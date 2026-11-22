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

# 让插件能 import mashiro（sdk/python 目录相对本文件定位，可被 MASHIRO_SDK 覆盖）
_SDK_DIR = os.environ.get(
    "MASHIRO_SDK", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.abspath(_SDK_DIR))

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


def call_api(method, params, plugin_id=None):
    """插件侧 SDK 调框架 API（经 transport 注入插件模块）。"""
    with _pending_lock:
        _api_id[0] += 1
        rid = _api_id[0]
        ev = threading.Event()
        _pending[rid] = {"ev": ev, "result": None, "error": None}
    frame = {"id": rid, "method": method, "params": params}
    if plugin_id:
        frame["plugin"] = plugin_id
    send(frame)
    if not ev.wait(15):
        raise TimeoutError(f"api {method} timeout")
    item = _pending.pop(rid, {})
    if item.get("error"):
        raise RuntimeError(item["error"])
    return item.get("result")


def make_transport(plugin_id):
    """注入到插件模块命名空间的 mashiro transport。"""
    def call(method, params):
        return call_api(method, params, plugin_id)

    def register(handler, fn):
        handlers[(plugin_id, handler)] = fn
    return {"call_api": call, "register": register}


def load_plugin(params):
    pid, pdir, entry = params["id"], params["dir"], params["entry"]
    unload_plugin({"id": pid})  # 同 id 热重载：先清掉旧 handler 注册
    spec = importlib.util.spec_from_file_location(f"mashiro_plugin_{pid}", f"{pdir}/{entry}")
    mod = importlib.util.module_from_spec(spec)
    mod.__dict__["__mashiro_transport"] = make_transport(pid)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    plugins[pid] = mod
    return {"ok": True}


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
}


def main():
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
