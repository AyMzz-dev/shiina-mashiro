# 椎名真白 · 插件规范

插件 = `plugin.json` + 一个入口文件。给 Agent 生成用，保持最小形态。

## 目录结构

```
my-plugin/
├─ plugin.json
└─ main.js | main.py
```

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

> 权限是**强制校验**的：插件调用未声明的能力会被框架拒绝（返回 permission denied）。
> `send` 覆盖 send_message/send_markdown；`media` → send_media；`recall` → recall；
> `group_admin` → guild_info/get_member_list/get_member_info/kick_member；
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

## ctx 上下文

| 字段 | 说明 |
|---|---|
| appid / kind / group_id / user / user_int / nickname / text / msg_id | 事件信息 |
| ctx.reply(text) | 被动回复（带 msg_id，无频次限制） |
| ctx.send(text) | 主动发送 |
| ctx.kv.get/set/delete/incr | 插件私有 KV |

## 插件 API

| 分类 | 方法 | 参数要点 |
|---|---|---|
| 消息 | `send_message` `{appid,kind,openid,text}` `send_markdown` `{...content}` `send_media` `{...file_type,url}` `recall` `{...msg_id}` | kind 0群/1频道/2私聊/3频道私聊 |
| 存储 | `kv_get` `kv_set` `kv_del` `kv_incr` `{key,...}` | 按 plugin id 隔离 |
| 定时 | `add_timer` `{delay_secs,repeat_secs,remark}` → timer_id；`cancel_timer` `{timer_id}` | 触发 `event:timer` handler |
| 查询 | `bot_list` `get_short_id` `{appid,user}` `get_openid` `{appid,short_id}` | 用户整数短 ID 双向映射 |
| 频道管理 | `guild_info` `get_member_list` `get_member_info` `kick_member` `{appid,guild_id,[user_id],[delete_msg_days]}` | 仅 guild 域；QQ 群无官方群管 API |
| 其他 | `log` `{msg,appid?}` `http_request` `{method,url,headers?,body?}` `ping` | |

### 发送可选参数（三个 send_* 通用）

| 字段 | 说明 |
|---|---|
| `msg_id` | 被动回复（SDK 的 `reply*` 自动带），有频次豁免 |
| `keyboard` | 内联键盘 payload `{"content":{"rows":[...]}}`，原样并入消息体 |
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
