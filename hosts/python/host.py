#!/usr/bin/env python
"""
Mashiro Python 插件宿主。

与框架之间走 stdin/stdout NDJSON（见 proto.rs）。
每个插件是独立 module，在独立线程里跑 handler；
插件调框架 API = 反向请求，由宿主转发（stdout 写加锁）。
"""
import importlib.util
import json
import os
import sys
import threading
import types

# 让插件能 import mashiro（sdk/python 目录相对本文件定位，可被 MASHIRO_SDK 覆盖）。
# 候选链：环境变量 → exe 旁 sdk/（打包布局）→ 向上爬目录找 workspace sdk/（dev）
def _resolve_sdk():
    if os.environ.get("MASHIRO_SDK"):
        return os.environ["MASHIRO_SDK"]
    cand = [os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "sdk", "python")]
    d = os.path.dirname(os.path.abspath(__file__))
    for _ in range(6):
        d = os.path.dirname(d)
        cand.append(os.path.join(d, "sdk", "python"))
    for p in cand:
        if os.path.isdir(p):
            return p
    return cand[0]


sys.path.insert(0, os.path.abspath(_resolve_sdk()))

plugins = {}          # id -> module
handlers = {}         # (plugin_id, handler) -> callable
_pending = {}         # api_req_id -> threading.Event + result
_pending_lock = threading.Lock()
_write_lock = threading.Lock()
_api_id = [0]


def send(obj):
    with _write_lock:
        sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
        sys.stdout.flush()


def respond(req_id, result=None, error=None):
    send({"id": req_id, "result": result, "error": error})


def call_api(method, params, plugin_id=None, token=None):
    """插件侧 SDK 调框架 API（经 transport 注入插件模块）。"""
    with _pending_lock:
        _api_id[0] += 1
        rid = _api_id[0]
        ev = threading.Event()
        _pending[rid] = {"ev": ev, "result": None, "error": None}
    frame = {"id": rid, "method": method, "params": params}
    if plugin_id:
        frame["plugin"] = plugin_id
        frame["token"] = token
    send(frame)
    if not ev.wait(15):
        raise TimeoutError(f"api {method} timeout")
    item = _pending.pop(rid, {})
    if item.get("error"):
        raise RuntimeError(item["error"])
    return item.get("result")


def make_transport(plugin_id, token):
    """注入到插件模块命名空间的 mashiro transport。"""
    def call(method, params):
        return call_api(method, params, plugin_id, token)

    def register(handler, fn):
        handlers[(plugin_id, handler)] = fn
    return {"call_api": call, "register": register}


_load_lock = threading.Lock()


def load_plugin(params):
    """exec_module 可能慢/阻塞 → 由调用方放独立线程跑，别卡协议主循环。"""
    with _load_lock:
        pid, pdir, entry = params["id"], params["dir"], params["entry"]
        unload_plugin({"id": pid})  # 同 id 热重载：先清掉旧 handler 注册
        # 插件目录进 sys.path：多文件布局下入口可以 import 同目录辅助模块
        _pdir = os.path.abspath(pdir)
        if _pdir not in sys.path:
            sys.path.insert(0, _pdir)
        spec = importlib.util.spec_from_file_location(f"mashiro_plugin_{pid}", f"{pdir}/{entry}")
        mod = importlib.util.module_from_spec(spec)
        mod.__dict__["__mashiro_transport"] = make_transport(pid, params.get("token", ""))
        sys.modules[spec.name] = mod
        spec.loader.exec_module(mod)
        plugins[pid] = mod
        return {"ok": True}


def _load_async(req_id, params):
    try:
        respond(req_id, result=load_plugin(params))
    except Exception as e:  # noqa: BLE001
        respond(req_id, error=str(e))


def unload_plugin(params):
    pid = params["id"]
    plugins.pop(pid, None)
    for k in [k for k in handlers if k[0] == pid]:
        del handlers[k]
    return {"ok": True}


def invoke(params):
    fn = handlers.get((params["plugin"], params["handler"]))
    if fn:
        threading.Thread(target=_safe_call, args=(fn, params["event"]), daemon=True).start()
    return {"accepted": True}


def invoke_batch(params):
    """单帧扇出：同帧 items 逐个找 handler 后台跑，事件只传一次。"""
    event = params.get("event")
    delivered = 0
    for it in params.get("items") or []:
        fn = handlers.get((it.get("plugin"), it.get("handler")))
        if fn:
            threading.Thread(target=_safe_call, args=(fn, event), daemon=True).start()
            delivered += 1
    return {"delivered": delivered}


def _safe_call(fn, event):
    try:
        fn(event)
    except Exception as e:  # noqa: BLE001
        call_api("log", {"level": "error", "msg": f"{type(e).__name__}: {e}"})


def describe(params):
    """返回插件已注册的 handler 清单（cmd:xxx / event:xxx）。"""
    pid = params["plugin"]
    return {"handlers": [h for (p, h) in handlers if p == pid]}


METHODS = {
    "ping": lambda p: "pong",
    "load_plugin": load_plugin,
    "unload_plugin": unload_plugin,
    "describe": describe,
    "invoke": invoke,
    "invoke_batch": invoke_batch,
}


def main():
    # Windows 下 stdin 默认用区域编码（GBK），框架写进来的是 UTF-8 NDJSON——
    # 路径含非 ASCII 字符（如中文桌面）时会 mojibake，强制按 UTF-8 解码
    try:
        sys.stdin.reconfigure(encoding="utf-8", errors="replace")
    except Exception:  # noqa: BLE001
        pass
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue

        # 框架回包（插件 API 调用的结果）
        if "id" in msg and "method" not in msg:
            with _pending_lock:
                item = _pending.get(msg["id"])
            if item:
                item["result"] = msg.get("result")
                item["error"] = msg.get("error")
                item["ev"].set()
            continue

        # 框架请求
        if "method" in msg and "id" in msg:
            if msg["method"] == "load_plugin":
                # 入口文件 exec 放独立线程：插件顶层代码阻塞不拖死协议循环
                threading.Thread(target=_load_async,
                                 args=(msg["id"], msg.get("params") or {}),
                                 daemon=True).start()
                continue
            fn = METHODS.get(msg["method"])
            if not fn:
                respond(msg["id"], error=f"method {msg['method']} not found")
                continue
            try:
                respond(msg["id"], result=fn(msg.get("params") or {}))
            except Exception as e:  # noqa: BLE001
                respond(msg["id"], error=str(e))
            continue

        # 框架通知
        if msg.get("type") == "shutdown":
            sys.exit(0)


if __name__ == "__main__":
    main()
