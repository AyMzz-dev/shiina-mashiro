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
    """频道(guild)域接口。"""

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


class _Group:
    """QQ 群管理（v2/groups，机器人需为群管理员；group_admin 权限）。"""

    def __init__(self, t, appid):
        self._t = t
        self._appid = appid

    def _p(self, group_openid, extra):
        p = {"appid": self._appid, "group_openid": group_openid}
        p.update(extra)
        return p

    def info(self, group_openid):
        return _call(self._t, "group_info", self._p(group_openid, {}))

    def members(self, group_openid, cursor=""):
        return _call(self._t, "group_members", self._p(group_openid, {"cursor": cursor}))

    def member(self, group_openid, member_openid):
        return _call(self._t, "group_member_info",
                     self._p(group_openid, {"member_openid": member_openid}))

    def blacklist(self, group_openid, cursor="", limit=20):
        return _call(self._t, "group_blacklist",
                     self._p(group_openid, {"cursor": cursor, "limit": limit}))

    def blacklist_op(self, group_openid, op, member_openids):
        """op: "add"|"del"，member_openids ≤20。"""
        return _call(self._t, "group_blacklist_op",
                     self._p(group_openid, {"op": op, "member_openids": list(member_openids)}))

    def kick(self, group_openid, member_openids):
        return _call(self._t, "group_remove_members",
                     self._p(group_openid, {"member_openids": list(member_openids)}))

    def mute_setting(self, group_openid):
        return _call(self._t, "group_mute_setting", self._p(group_openid, {}))

    def mute_members(self, group_openid, members):
        """members: [{"op":"add"|"update"|"del","member_openid":...,"mute_expire_at":"RFC3339"}] ≤20。"""
        return _call(self._t, "group_mute_members",
                     self._p(group_openid, {"members": members}))

    def join_requests(self, group_openid, cursor="", limit=20):
        return _call(self._t, "group_join_requests",
                     self._p(group_openid, {"cursor": cursor, "limit": limit}))

    def approve_join(self, group_openid, member_openid, op,
                     join_request_id=None, reject_reason=None, add_to_blacklist=False):
        """op: "approve"|"decline"；decline 可带 reject_reason/add_to_blacklist。"""
        return _call(self._t, "group_approve_join",
                     self._p(group_openid, {
                         "member_openid": member_openid, "op": op,
                         "join_request_id": join_request_id,
                         "reject_reason": reject_reason,
                         "add_to_member_blacklist": bool(add_to_blacklist)}))


class _Strategy:
    """入群自动审批策略（group_admin 权限）。"""

    def __init__(self, t, appid):
        self._t = t
        self._appid = appid

    def list(self, cursor="", limit=20):
        return _call(self._t, "strategy_list",
                     {"appid": self._appid, "cursor": cursor, "limit": limit})

    def create(self, **body):
        """body: group_openids?|group_ids?, is_enable?, expire_at?, remark?。"""
        return _call(self._t, "strategy_create", {"appid": self._appid, **body})

    def patch(self, strategy_id, **body):
        return _call(self._t, "strategy_patch",
                     {"appid": self._appid, "strategy_id": strategy_id, **body})

    def delete(self, strategy_id):
        return _call(self._t, "strategy_delete",
                     {"appid": self._appid, "strategy_id": strategy_id})

    def execute(self, strategy_id):
        return _call(self._t, "strategy_execute",
                     {"appid": self._appid, "strategy_id": strategy_id})

    def whitelist(self, strategy_id, op, users):
        """op: "add"|"del"；users 为 QQ 号码字符串列表（≤10000）。"""
        return _call(self._t, "strategy_whitelist",
                     {"appid": self._appid, "strategy_id": strategy_id,
                      "op": op, "whitelist_users": list(users)})


class _Menu:
    """私聊自定义菜单（仅 C2C 场景；group_admin 权限）。"""

    def __init__(self, t, appid):
        self._t = t
        self._appid = appid

    def get(self):
        return _call(self._t, "menu_get", {"appid": self._appid})

    def put(self, menu):
        """menu {"items":[...]}，覆盖式更新。"""
        return _call(self._t, "menu_put", {"appid": self._appid, "menu": menu})


class _Panel:
    """指令面板（group_admin 权限）。"""

    def __init__(self, t, appid):
        self._t = t
        self._appid = appid

    def list(self, scope, cursor="", limit=20):
        """scope: "c2c"|"group"|"channel"|"dm"。"""
        return _call(self._t, "panel_list",
                     {"appid": self._appid, "scope": scope, "cursor": cursor, "limit": limit})

    def create(self, **body):
        return _call(self._t, "panel_create", {"appid": self._appid, **body})

    def get(self, panel_id):
        return _call(self._t, "panel_get", {"appid": self._appid, "panel_id": panel_id})

    def update(self, panel_id, **body):
        return _call(self._t, "panel_update",
                     {"appid": self._appid, "panel_id": panel_id, **body})

    def delete(self, panel_id):
        return _call(self._t, "panel_delete",
                     {"appid": self._appid, "panel_id": panel_id})

    def target(self, panel_id, op, user_openids=None, group_openids=None):
        """op: "add"|"del"；c2c 传 user_openids，group 传 group_openids（≤20）。"""
        return _call(self._t, "panel_target",
                     {"appid": self._appid, "panel_id": panel_id, "op": op,
                      "user_openids": list(user_openids or []),
                      "group_openids": list(group_openids or [])})


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
        self.group = _Group(t, self.appid)
        self.strategy = _Strategy(t, self.appid)
        self.menu = _Menu(t, self.appid)
        self.panel = _Panel(t, self.appid)

    def _base(self, extra):
        p = {"appid": self.appid, "kind": self.kind, "openid": self.group_id}
        p.update(extra)
        return p

    def send(self, text, **opts):
        """opts 可带 event_id / msg_seq / is_wakeup / keyboard / recall_after_secs。"""
        return _call(self._t, "send_message", self._base({"text": text, **opts}))

    def reply(self, text, **opts):
        return _call(self._t, "send_message",
                     self._base({"text": text, "msg_id": self.msg_id, **opts}))

    def send_markdown(self, content, **opts):
        return _call(self._t, "send_markdown", self._base({"content": content, **opts}))

    def reply_markdown(self, content, **opts):
        return _call(self._t, "send_markdown",
                     self._base({"content": content, "msg_id": self.msg_id, **opts}))

    def send_media(self, file_type, url, **opts):
        """file_type: 1 图片 / 2 视频 / 3 语音 / 4 文件；url 直传。"""
        return _call(self._t, "send_media",
                     self._base({"file_type": file_type, "url": url,
                                 "msg_id": self.msg_id, **opts}))

    def send_file(self, file_type, path, **opts):
        """本地文件分片上传发送（群/单聊）。"""
        return _call(self._t, "send_media_file",
                     self._base({"file_type": file_type, "path": path,
                                 "msg_id": self.msg_id, **opts}))

    def send_card(self, title, url, desc="", pic_url="", **opts):
        """图文卡片（msg_type 8）：title/url 必填，desc/pic_url 可选。"""
        return _call(self._t, "send_card",
                     self._base({"title": title, "desc": desc, "url": url,
                                 "pic_url": pic_url, "msg_id": self.msg_id, **opts}))

    def send_stream(self, **fields):
        """单聊流式分块：input_mode/input_state/index/content_type/content_raw +
        event_id|msg_id，首块不带 stream_msg_id，用返回 id 续传。"""
        return _call(self._t, "send_stream", self._base(fields))

    def respond(self, code=0):
        """INTERACTION_CREATE 事件应答：0 成功/1 失败/2 频繁/3 重复/4 无权限/5 仅管理员。"""
        iid = (self.event.get("raw") or {}).get("id") or self.msg_id
        return _call(self._t, "interaction_respond",
                     {"appid": self.appid, "interaction_id": iid, "code": code})

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

    def log(self, msg, level="info"):
        """写一条插件日志到日志页（调试随手可用；level: info/warn/error）。"""
        return _call(self._module_t(), "log", {"msg": str(msg), "level": level})

    def config(self, key, default=None):
        """读可视化配置（plugin.json config_schema 声明的 key）；未设置返回 schema 默认值，再兜底 default。"""
        v = _call(self._module_t(), "config_get", {"key": key})
        return default if v is None else v

    def config_all(self):
        """全部配置一次拿完：{key: value}。"""
        return _call(self._module_t(), "config_all")

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

    # ---------- 群管 / 策略 / 菜单 / 面板 / 用户（group_admin 权限） ----------

    def group(self, appid):
        """QQ 群管理访问器：bot.group(appid).info(group_openid) 等。"""
        return _Group(self._module_t(), appid)

    def strategy(self, appid):
        return _Strategy(self._module_t(), appid)

    def menu(self, appid):
        return _Menu(self._module_t(), appid)

    def panel(self, appid):
        return _Panel(self._module_t(), appid)

    def guild(self, appid):
        return _Guild(self._module_t(), appid)

    def user_info(self, appid, user_openid):
        return _call(self._module_t(), "user_info",
                     {"appid": appid, "user_openid": user_openid})

    def share_link(self, appid, callback_data=""):
        """生成加好友分享链接（callback_data ≤32 字符）。"""
        return _call(self._module_t(), "generate_url_link",
                     {"appid": appid, "callback_data": callback_data})

    def respond_interaction(self, appid, interaction_id, code=0):
        return _call(self._module_t(), "interaction_respond",
                     {"appid": appid, "interaction_id": interaction_id, "code": code})

    def send_stream(self, appid, user_openid, **fields):
        """无事件上下文的单聊流式分块发送。"""
        return _call(self._module_t(), "send_stream",
                     {"appid": appid, "openid": user_openid, **fields})
