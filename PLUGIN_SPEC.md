# 椎名真白 · 插件规范

插件 = `plugin.json` + 入口文件（+ 可选辅助文件，多文件布局）。小插件单文件即可，代码量大时拆分。

## 目录结构

```
my-plugin/
├─ plugin.json
├─ main.js | main.py      # 入口
├─ store.js | store.py    # 可选：辅助文件
└─ lib/util.js            # 可选：支持子目录
```

> 多文件互相引入：
> - JS：`require("./store.js")`（CJS，相对入口所在目录解析；`require("mashiro")` 在任何文件里都可用）
> - Python：`import store`（插件目录已加入 sys.path）

## plugin.json

```json
{
  "id": "daily-signin",
  "name": "每日签到",
  "version": "1.0.0",
  "runtime": "python",
  "entry": "main.py",
  "description": "签到得积分",
  "author": "agent",
  "permissions": ["send", "kv", "timer"]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| id | ✅ | 全局唯一，规则/配置/KV 都按它归口 |
| name | ✅ | 界面显示名 |
| version | ✅ | semver |
| runtime | ✅ | `node` 或 `python` |
| entry | ✅ | 入口文件名 |
| description / author | | 展示用 |
| permissions | | 能力声明白名单：`send` `recall` `media` `kv` `timer` `http` `group_admin` |
| config_schema | | 可视化配置声明（见下） |

## config_schema（可视化配置）

声明后插件页出现「配置」按钮，自动渲染表单；值存插件 KV，SDK 用 `bot.config(key)` / `bot.config_all()` 读（Python 同名方法），未设置时返回 `default`。

```json
"config_schema": [
  { "key": "api_key", "label": "API 密钥", "ty": "string", "password": true, "desc": "第三方接口密钥" },
  { "key": "points", "label": "每次签到积分", "ty": "number", "default": 10 },
  { "key": "enable_kick", "label": "启用自动踢出", "ty": "bool", "default": true },
  { "key": "mode", "label": "验证方式", "ty": "select", "options": ["按钮", "答题"], "default": "按钮" }
]
```

| 字段 | 说明 |
|---|---|
| key / label | 配置键 / 表单显示名 |
| ty | `string`（默认）`textarea` `password` `number` `slider` `bool` `select` `radio` `multi_select` `tags` `path` `file` `color` `date` `time` |
| default | 默认值（multi_select/tags 用数组） |
| options | select / radio / multi_select 的候选 |
| desc | 说明文字 |
| password | 掩码输入（密钥类） |
| min / max / step | number / slider 的数值范围与步长 |
| placeholder | 输入占位提示 |
| required | 必填校验（保存时拦空值） |

> 权限是**强制校验**的：插件调用未声明的能力会被框架拒绝（返回 permission denied）。
> `send` → send_message/send_markdown/send_card/send_stream/interaction_respond；
> `media` → send_media/send_media_file/material_*；`recall` → recall；
> `group_admin` → guild_*/get_member_*/kick_member 及全部 QQ 群管（group_*/strategy_*/menu_*/panel_*/user_info/generate_url_link）；
> `timer` → add_timer/cancel_timer；`kv` → kv_*；`http` → http_request。
> ping/log/bot_list/get_short_id/get_openid 为本地只读接口，无需声明。

## 传输协议（宿主 ↔ 框架）

宿主进程 stdin/stdout，**NDJSON**（一行一个 JSON，UTF-8）。

```
框架 → 宿主请求   {"id":N,"method":"invoke","params":{...}}   → 回包 {"id":N,...}
宿主 → 框架回包   {"id":N,"result":{...},"error":null}
插件 → 框架 API   {"id":N,"method":"send_message","params":{}} → 框架回包同 id
通知             {"type":"register"|"log"|"shutdown","data":{...}}
```

宿主侧框架方法：`ping` / `load_plugin{id,dir,entry}` / `unload_plugin{id}` / `describe{plugin}`（返回插件已注册 handler 清单）/ `invoke{plugin,handler,event}`。

插件 → 框架的 API 调用帧可带 `"plugin":"<id>"` 标注来源（宿主自动注入，插件无需手写）。

handler 命名：`cmd:<指令名>`（消息前缀命中即触发，注册即生效）、`event:<事件类型>`（事件订阅，如 `event:timer`）。

## 完整示例（照着这个结构写）

```json
// plugin.json
{ "id": "sign-in", "name": "每日签到", "version": "1.0.0",
  "runtime": "node", "entry": "main.js",
  "permissions": ["send", "kv", "timer"] }
```

```js
// main.js —— Node 插件 = CommonJS，require("mashiro") 拿 SDK
const bot = require("mashiro");

// 指令：消息文本以「签到」开头触发（用户打 /签到 也能匹配）
bot.command("签到", async (ctx) => {
  const n = await ctx.kv.incr(`score:${ctx.user}`);
  await ctx.reply(`✅ ${ctx.nickname || "你"} 签到成功，积分 ${n}`);
});

// 订阅事件：event:<事件名>，timer 为框架定时器
bot.on("GROUP_MEMBER_ADD", async (ctx) => {
  await ctx.send(`欢迎新成员 ${ctx.user}`);
});

// 定时任务：10 秒后首次触发，之后每 3600s 一次；触发走 bot.on("timer")
bot.timer(10, 3600, "hourly-tick");
bot.on("timer", async (ctx) => {
  await bot.log("tick");
});

// INTERACTION_CREATE（按钮/回调点击）：ctx.respond(0) 应答
bot.on("INTERACTION_CREATE", async (ctx) => {
  await ctx.respond(0);
});
```

## 插件 SDK（sdk/js/mashiro.js · sdk/python/mashiro.py）

```js
// main.js
const bot = require("mashiro");
bot.command("签到", async (ctx) => {
  const n = await ctx.kv.incr("score");
  await ctx.reply(`签到成功，积分 ${n}`);
});
```

```python
# main.py
import mashiro
bot = mashiro.Bot()

@bot.command("签到")
def signin(ctx):
    n = ctx.kv.incr("score")
    ctx.reply(f"签到成功，积分 {n}")
```

## ctx 上下文（handler 参数）

字段：`appid / kind / groupId / user / userInt / nickname / text / msgId / atMe / memberRole`（memberRole: 0群主 1管理 2成员 -1未知）

| 方法 | 说明 |
|---|---|
| ctx.reply(text, opts?) | 被动回复（带 msg_id，无频次限制） |
| ctx.send(text, opts?) | 主动发送到当前会话 |
| ctx.sendMarkdown(content) / replyMarkdown | markdown 消息（按钮/卡片模板） |
| ctx.sendMedia / sendFile / sendCard / sendStream | 富媒体 / 本地文件 / 图文卡片 / 流式 |
| ctx.respond(code) | INTERACTION_CREATE 应答：0成功 1失败 2频繁 3重复 4无权限 5仅管理员 |
| ctx.recall(msgId?) | 撤回消息 |
| ctx.log(msg) | 写插件日志（当前事件 appid） |
| ctx.kv.get/set/del/incr | 插件私有 KV（值为字符串，JSON 自己序列化） |

## bot 模块级 API

| 方法 | 说明 |
|---|---|
| bot.command(name, fn) | 注册指令，handler=`cmd:<name>` |
| bot.on(type, fn) | 订阅事件，handler=`event:<type>`（含 "timer"） |
| bot.timer(delay, repeat=0, remark="") / cancelTimer(id) | 定时任务（timer 权限）；触发时事件载荷 `{timer_id, remark, fired_at}`，`ctx.event.remark` 或 `ctx.event.raw.remark` 都能拿到 |
| bot.log(msg, level) | 写插件日志 |
| bot.config(key) / configAll() | 读可视化配置（未设→default） |
| bot.bots() / shortId / openidOf / userInfo / shareLink | 账号与资料 |
| bot.group.* | info/members/kick/mute*/blacklist*/joinRequests/approveJoin（group_admin） |
| bot.guild.* | info/members/member/kick（频道域） |
| bot.strategy.* | 入群自动审批策略（group_admin） |
| bot.menu.* / bot.panel.* | 私聊菜单 / 指令面板（group_admin） |
| bot.material.* | markdown 图片素材（media） |
| bot.http(opts) | 通用 HTTP → {status, body}（http 权限） |

## 插件 API

| 分类 | 方法 | 参数要点 |
|---|---|---|
| 消息 | `send_message` `{appid,kind,openid,text}` `send_markdown` `{...content}` `send_media` `{...file_type,url}` `send_card` `{...title,desc?,url,pic_url?}` `send_media_file` `{...file_type,path}` `recall` `{...msg_id}` | kind 0群/1频道/2私聊/3频道私聊；send_card 为 msg_type 8 图文卡片；send_media_file 本地文件分片上传（仅群/单聊） |
| 流式 | `send_stream` `{appid,openid,input_mode,input_state,index,content_type,content_raw,event_id\|msg_id,stream_msg_id?,msg_seq?}` | 仅单聊；input_state 1 生成中/10 结束，首块不带 stream_msg_id、返回 id 续传 |
| 互动 | `interaction_respond` `{appid,interaction_id,code}` | INTERACTION_CREATE（按钮/菜单回调）应答；interaction_id 取事件 raw.id |
| 存储 | `kv_get` `kv_set` `kv_del` `kv_incr` `{key,...}` | 按 plugin id 隔离 |
| 定时 | `add_timer` `{delay_secs,repeat_secs,remark}` → timer_id；`cancel_timer` `{timer_id}` | 触发 `event:timer` handler |
| 查询 | `bot_list` `get_short_id` `{appid,user}` `get_openid` `{appid,short_id}` | 用户整数短 ID 双向映射 |
| QQ 群管 | `group_info` `group_members` `group_member_info` `{appid,group_openid,...}` `group_blacklist` `group_blacklist_op` `group_remove_members` `group_mute_setting` `group_mute_members` `group_join_requests` `group_approve_join` | 需机器人为群管理员；黑名单/禁言 op 均为 add/del（禁言另有 update），mute_expire_at 为 RFC3339，最长 30 天 |
| 审批策略 | `strategy_list` `strategy_create` `strategy_patch` `strategy_delete` `strategy_execute` `strategy_whitelist` `{appid,strategy_id?...}` | 入群自动审批：白名单 QQ 号命中即自动通过；execute 全量扫描（约 10 分钟） |
| 频道管理 | `guild_info` `get_member_list` `get_member_info` `kick_member` `{appid,guild_id,[user_id],[delete_msg_days]}` | 仅 guild 域 |
| 菜单/面板 | `menu_get` `menu_put` `{appid,menu}` `panel_list` `panel_create` `panel_get` `panel_update` `panel_delete` `panel_target` `{appid,panel_id?,scope?,op?,...}` | 菜单仅 C2C 场景；panel scope = c2c/group/channel/dm |
| 用户/分享 | `user_info` `{appid,user_openid}` `generate_url_link` `{appid,callback_data}` | 用户资料 / 加好友分享链接（callback_data ≤32 字符） |
| 其他 | `log` `{msg,appid?}` `http_request` `{method,url,headers?,body?}` `ping` | |

### 发送可选参数（send_* 通用）

| 字段 | 说明 |
|---|---|
| `msg_id` | 被动回复（SDK 的 `reply*` 自动带），有频次豁免 |
| `event_id` | 被动回复事件（INTERACTION_CREATE/GROUP_ADD_ROBOT/GROUP_MSG_RECEIVE），与 msg_id 二选一 |
| `msg_seq` | 同 msg_id 多次回复的去重序号（默认 1，重复组合会失败） |
| `is_wakeup` | true = 互动召回消息（30 天 4 周期内免 msg_id 主动触达，有额度） |
| `keyboard` | 内联键盘 payload `{"id":"tpl"}` 或 `{"content":{"rows":[...]}}`，原样并入消息体 |
| `recall_after_secs` | >0 = 发送 N 秒后自动撤回（主动消息取返回 id 撤） |

### 事件订阅

`event:<原始事件名>` handler 收到对应类型的全部事件（如 `event:GROUP_AT_MESSAGE_CREATE`、`event:GROUP_ADD_ROBOT`、`event:timer`）。
事件投递为标准化 MessageEvent；非消息类事件 text 为空，不会命中任何 cmd 指令前缀。

## MessageEvent（投递给插件的事件）

```json
{
  "appid": 1234567890,
  "kind": 0,
  "event_type": "GROUP_AT_MESSAGE_CREATE",
  "group_id": "...", "user": "...", "nickname": "...",
  "msg_id": "...", "text": "...", "reply_to": "",
  "at_me": true, "member_role": 2, "user_int": 42,
  "raw": {}
}
```

`kind`: 0 群 / 1 频道 / 2 私聊 / 3 频道私聊。

## 安装方式

- **目录安装**：`install_plugin` → 指向含 plugin.json 的目录
- **脚手架**：`scaffold_plugin{id,name,runtime}` → 生成模板并直接安装，Agent 生成插件的落点

## Webhook 接入（可选）

设置 `webhook_listen`（如 `0.0.0.0:8464`）后框架起 HTTP 服务：
`POST /qq/callback/{appid}` → Ed25519 验签 → op13 回签 / op0 事件进同一分发器。
默认关闭（纯 WS 模式）。改设置后重启生效。
