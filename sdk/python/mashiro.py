"""
Mashiro Python 插件 SDK。
在 host.py 里运行：host 把 __mashiro_transport 注入**插件模块自己的命名空间**，
SDK 在装饰器里从被装饰函数的 __globals__ 取回它（天然按插件隔离）。

插件写法：
    import mashiro
    bot = mashiro.Bot()

    @bot.command("签到")
    def signin(ctx):
        n = ctx.kv.incr("score")
        ctx.reply(f"签到成功，积分 {n}")

    @bot.on("timer")
    def tick(ctx):
        ctx.send("整点报时")
"""


def _transport_of(fn):
    """被装饰函数所属模块的 globals 里找 host 注入的 transport。"""
    t = fn.__globals__.get("__mashiro_transport")
    if t is None:
        raise RuntimeError("mashiro 插件必须运行在 host.py 宿主内")
    return t


def _call(t, method, params=None):
    return t["call_api"](method, params or {})


class _Kv:
    def __init__(self, t):
        self._t = t

    def get(self, key):
        return _call(self._t, "kv_get", {"key": key})

    def set(self, key, value):
        return _call(self._t, "kv_set", {"key": key, "value": str(value)})

    def delete(self, key):
        return _call(self._t, "kv_del", {"key": key})

    def incr(self, key):
        return _call(self._t, "kv_incr", {"key": key})


class _Guild:
    """频道(guild)域接口（QQ 群无官方群管 API）。"""

    def __init__(self, t, appid):
        self._t = t
        self._appid = appid

    def info(self, guild_id):
        return _call(self._t, "guild_info", {"appid": self._appid, "guild_id": guild_id})

    def members(self, guild_id, limit=100, after="0"):
        return _call(self._t, "get_member_list",
                     {"appid": self._appid, "guild_id": guild_id, "limit": limit, "after": after})

    def member(self, guild_id, user_id):
        return _call(self._t, "get_member_info",
                     {"appid": self._appid, "guild_id": guild_id, "user_id": user_id})

    def kick(self, guild_id, user_id, delete_msg_days=0):
        return _call(self._t, "kick_member",
                     {"appid": self._appid, "guild_id": guild_id, "user_id": user_id,
                      "delete_msg_days": delete_msg_days})


class Ctx:
    """事件上下文：字段 + 便捷回复/发送/KV。"""

    def __init__(self, event, t):
        self._t = t
        self.event = event
        self.appid = event.get("appid")
        self.kind = event.get("kind")
        self.group_id = event.get("group_id")
        self.guild_id = event.get("guild_id")
        self.user = event.get("user")
        self.user_int = event.get("user_int")
        self.nickname = event.get("nickname")
        self.text = event.get("text")
        self.msg_id = event.get("msg_id")
        self.at_me = event.get("at_me")
        self.member_role = event.get("member_role")
        self.kv = _Kv(t)
        self.guild = _Guild(t, self.appid)

    def _base(self, extra):
        p = {"appid": self.appid, "kind": self.kind, "openid": self.group_id}
        p.update(extra)
        return p

    def send(self, text):
        return _call(self._t, "send_message", self._base({"text": text}))

    def reply(self, text):
        return _call(self._t, "send_message", self._base({"text": text, "msg_id": self.msg_id}))

    def send_markdown(self, content):
        return _call(self._t, "send_markdown", self._base({"content": content}))

    def reply_markdown(self, content):
        return _call(self._t, "send_markdown", self._base({"content": content, "msg_id": self.msg_id}))

    def send_media(self, file_type, url):
        """file_type: 1 图片 / 2 视频 / 3 语音 / 4 文件；url 直传。"""
        return _call(self._t, "send_media",
                     self._base({"file_type": file_type, "url": url, "msg_id": self.msg_id}))

    def recall(self, msg_id=None):
        return _call(self._t, "recall", self._base({"msg_id": msg_id or self.msg_id}))

    def log(self, msg):
        return _call(self._t, "log", {"appid": self.appid, "msg": msg})


class Bot:
    def command(self, name):
        """注册指令：框架按用户配置的触发词匹配后投递 cmd:<name>。"""
        def deco(fn):
            t = _transport_of(fn)
            t["register"](f"cmd:{name}", lambda ev: fn(Ctx(ev, t)))
            return fn
        return deco

    def on(self, event_type):
        """订阅事件：handler = event:<type>（如 "timer"）。"""
        def deco(fn):
            t = _transport_of(fn)
            t["register"](f"event:{event_type}", lambda ev: fn(Ctx(ev, t)))
            return fn
        return deco

    # ---------- 无事件上下文也可调（经装饰器拿 transport） ----------

    def timer(self, delay_secs, repeat_secs=0, remark=""):
        """定时器装饰器：注册 event:timer handler 并落一条定时任务。

        @bot.timer(3600, 3600, "每小时")
        def tick(ctx): ...
        """
        def deco(fn):
            t = _transport_of(fn)
            t["register"]("event:timer", lambda ev: fn(Ctx(ev, t)))
            _call(t, "add_timer",
                  {"delay_secs": delay_secs, "repeat_secs": repeat_secs, "remark": remark})
            return fn
        return deco

    @staticmethod
    def _module_t():
        """bot.http 等模块级调用：从调用方模块 globals 找 transport。"""
        import inspect
        frame = inspect.currentframe()
        try:
            # _module_t → 包一层 staticmethod 的 frame 跳过
            caller = frame.f_back.f_back if frame and frame.f_back else None
            while caller:
                t = caller.f_globals.get("__mashiro_transport")
                if t is not None:
                    return t
                caller = caller.f_back
            raise RuntimeError("找不到 transport（请在插件模块作用域内调用）")
        finally:
            del frame

    def cancel_timer(self, timer_id):
        return _call(self._module_t(), "cancel_timer", {"timer_id": timer_id})

    def bots(self):
        return _call(self._module_t(), "bot_list")

    def short_id(self, appid, user):
        return _call(self._module_t(), "get_short_id", {"appid": appid, "user": user})

    def openid_of(self, appid, short_id):
        return _call(self._module_t(), "get_openid", {"appid": appid, "short_id": short_id})

    def http(self, method, url, headers=None, body=None):
        return _call(self._module_t(), "http_request",
                     {"method": method, "url": url, "headers": headers or {}, "body": body})

    # ---------- Markdown 图片素材（media 权限） ----------

    def material_list(self, appid, cursor=""):
        return _call(self._module_t(), "material_list", {"appid": appid, "cursor": cursor})

    def material_upload(self, appid, path):
        """本地图片 → qqbot-media:// 素材链接（png/jpg/gif/webp，<=10MB）"""
        return _call(self._module_t(), "material_upload", {"appid": appid, "path": path})

    def material_delete(self, appid, image_id):
        return _call(self._module_t(), "material_delete", {"appid": appid, "image_id": image_id})
