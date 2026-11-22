# 椎名真白 Shiina Mashiro

> 《纯白、无口、但群里每句话都记得》

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows-lightgrey.svg)](#开发)
[![Core](https://img.shields.io/badge/core-Rust%20%2B%20Tauri%202-orange.svg)](#结构)
[![UI](https://img.shields.io/badge/ui-React%2018%20%2B%20Vite-61dafb.svg)](#结构)

基于 **Rust + Tauri 2** 的轻量级 QQ 机器人桌面框架 —— 纯白铃铛（Qt/C++）的重写瘦身版。

框架只做管道：**收消息 → 指令前缀 / 事件订阅命中 → 投递插件 → 发消息**。
抽奖、商店、AI、签到等一切业务都属于插件——「生成」页内置 Agent 工作台：一句话需求 → LLM 按 `PLUGIN_SPEC.md` 产出插件 → 真实加载校验 → 一键上线 / 热重载。

## 特性

- **多账号托管**：QQ 官方机器人协议，账号级 WS / Webhook 双模式、沙盒环境、群 / 频道 Markdown 独立开关、intents 自定义订阅（12 个订阅位可选）；**扫码一键登录**——本地生成 AES-256-GCM 密钥，扫码后轮询解密回填 secret，secret 不出本机
- **插件生态**：JS（每插件独立 Node Worker 线程）/ Python（独立宿主进程，每插件独立模块）双运行时，stdin/stdout NDJSON 行协议；`cmd:签到` 注册即生效（消息前缀命中直接投递），`event:xxx` 订阅全量事件流（含 `event:timer` 定时）
- **消息能力**：文本 / Markdown（按域开关自动降级纯文本）/ 富媒体（图/视/语音/文件 URL 直传）/ 内联键盘 / 定时自动撤回 / 消息撤回；被动回复带 msg_id 享频次豁免
- **插件管理**：目录导入（系统目录选择器直装）、脚手架生成；启停 / 重载 / 卸载 / 打开目录；按账号授权（勾选哪些 bot 可用该插件）；handler 清单可见
- **权限门**：`plugin.json` 的 `permissions` 是强制白名单——`send` `recall` `media` `kv` `timer` `http` `group_admin`，未声明的能力调用直接被拒
- **聊天页**：会话列表 + 历史消息回看（分页）+ 手动发送（文本 / Markdown / 内联键盘可视化编辑）；群内授权状态查询 + 一键发送「授权指引卡」
- **任务页**：插件注册的全部定时任务（一次性 / 周期），启停 / 删除 / 到期时间
- **素材页**：Markdown 图片素材管理（`qqbot-media://` 引用），上传（png/jpg/gif/webp ≤10MB，prepare→直传→complete 三段式）/ 列表分页 / 删除 / 剩余天数
- **Agent 生成**（`生成` 页）：OpenAI 兼容 / Anthropic 双协议；模型经**原生工具调用**（write_file / read_file / delete_file / list_files / install_plugin）自主写文件、安装校验、读报错修复，最多 16 轮；产出写盘后经真实宿主 `load + describe` 验证，同 id 再装即热重载；不支持工具的接口退化 `<<<FILE:>>>` 块解析兜底
- **MCP 调试服务**：开箱即用（默认 `127.0.0.1:8465`，Bearer 鉴权 token 自动生成），AI 客户端连上即可读写插件文件、热重载、触发任意 handler、查日志 / KV / 定时器、发测试消息——「远程调插件」闭环
- **日志**：统一日志表（接收 / 发送 / 事件 / 错误 / 插件 / 系统，raw 存完整 JSON 可查看报文），分类 / 账号过滤，按天数自动清理（默认 30 天）
- **存储**：SQLite 单库（账号 / 插件 / 定时器 / KV / 日志 / 设置 / uid 映射 / 插件日统计）；secret 经 Windows DPAPI（当前用户域）加密落库，列表只回掩码
- **安全细节**：插件权限白名单、`plugin_allowed` 账号授权门、Agent/MCP 写文件路径白名单（拒绝 `..`/穿越）、webhook Ed25519 验签 + op13 回签、MCP Bearer 鉴权、网关 (appid, msg_id) 去重防双投递

## 界面

首页（统计卡 + 24h 收发柱状 + 今日插件 TOP）· 账号（扫码/手动、启停、沙盒/Markdown/intents）· 插件 · 聊天 · 任务 · 素材 · 生成 · 日志 · 设置 · 关于 —— 共 10 页，窗口固定 1121×655 无边框。

## 结构

```
crates/mashiro-core/    核心库（不依赖 Tauri，可独立测试）
  src/bot/              QQ 协议：gateway(WS 重连/Resume) api token bind(扫码) client media
  src/router/           事件标准化(parse) + 分发(去重→日志→cmd 命中→event 扇出)
  src/plugin/           manifest / proto(NDJSON) / host(子进程读写) / manager / sdk_api(权限门+API)
  src/storage/          db(SQLite) / kv / models / secret(DPAPI)
  src/agent.rs          LLM 插件生成（工具调用循环 + <<<FILE:>>> 兜底）
  src/scheduler.rs      定时任务轮询（15s 扫到期 → 投递 event:timer）
  src/webhook.rs        Webhook 回调服务（Ed25519 验签 / op13 回签）
  src/mcp.rs            MCP 调试服务（Streamable HTTP，Bearer 鉴权）
src-tauri/              Tauri 壳（43 个 commands / AppState 装配 / 自启与旧数据迁移）
ui/                     React 18 + TypeScript + Vite + antd 前端（10 页）
hosts/                  插件宿主进程：node/host.js + worker.js · python/host.py
sdk/                    插件 SDK：js/mashiro.js · python/mashiro.py
plugins/                示例插件 hello-js / hello-py
tests/                  宿主协议冒烟测试（host-smoke.js）
PLUGIN_SPEC.md          插件规范（也是 Agent 生成的输入）
```

事件管线：

```
QQ 平台 ──WS op0 / webhook op0──▶ router::parse 标准化
                                   │
                            dispatch（去重·写日志·短ID填充）
                                   │
              ┌──── cmd:<前缀> 命中 ──┤──── event:<事件名> 扇出
              ▼                     ▼
        PluginHost(Node/Python 子进程, NDJSON)
              │  invoke{plugin,handler,event}
              ▼
        插件 handler ──▶ 反向调框架 API（send/kv/timer/http…，权限门校验）
```

## 开发

前置：**Rust stable (MSVC)**、**Node 18+**、VS C++ Build Tools、WebView2；跑 Python 插件另需 Python 3.x。

```bash
npm install               # 仓库根 package.json 即前端依赖
npm run tauri dev         # 一条命令同时起 Vite + 编译壳 + 开窗口
```

其他命令：

| 命令 | 作用 |
|---|---|
| `npm run dev` | 只起前端 Vite（:1420） |
| `npm run build` | 打包前端到 `ui/dist` |
| `npm run smoke` | Node 宿主协议冒烟测试 |
| `node tests/host-smoke.js python` | Python 宿主协议冒烟测试（不经 Rust 直接驱动 host） |
| `cargo check --workspace` | Rust 编译检查 |

## 打包

```bash
npm run tauri build       # 产出 NSIS 安装包 + ShiinaMashiro.exe（bundle id ai.shiina.mashiro）
```

`hosts/` 目录作为 resource 打进安装包；运行时宿主目录定位顺序：`MASHIRO_HOSTS_DIR` 环境变量 → exe 旁 `hosts/` → macOS `Contents/Resources/hosts` → 开发期 `./hosts`。

## CI 发布

`.github/workflows/release.yml`：打 `v*` tag（或手动触发）即三平台并行构建并创建 GitHub Release——Windows 出 NSIS `.exe`、macOS 出 universal `.dmg`（Apple Silicon + Intel 单包）、Ubuntu 22.04 出 `.deb` + `.AppImage`。

```bash
git tag v0.1.0 && git push --tags
```

## 数据

数据目录 `<程序运行目录>/data/`：`mashiro.db`（SQLite）+ `plugins/`（插件落地）。旧 `%APPDATA%/mashiro` 的库与插件目录首启自动复制迁移并改写路径，老库增量补列（`PRAGMA table_info` 缺列即 `ALTER TABLE`）。

## 设置项

「设置」页写入 `settings` 表，关键项：

| key | 说明 |
|---|---|
| `node_path` / `python_path` | 宿主解释器路径；留空按 PATH 找 `node` / `python`，「检测」按钮跑 `--version` 验证 |
| `webhook_listen` | Webhook 监听地址（如 `0.0.0.0:8464`）；留空 = 纯 WS 模式，改后重启生效 |
| `mcp_listen` | MCP 监听；留空自动 `127.0.0.1:8465`，填 `off` 关闭，改后重启生效 |
| `mcp_token` | MCP Bearer token；留空启动时自动生成并写回 |
| `log_keep_days` | 日志保留天数（默认 30，1–3650），每天自动清理 |
| `agent_provider` / `agent_base` / `agent_key` / `agent_model` | Agent 模型接入：`openai` / `anthropic`；OpenAI 兼容填到 `/v1` 或完整 `…/chat/completions` 均可，支持拉取模型列表与连通测试 |

## Webhook 接入（可选）

设置 `webhook_listen` 后起 HTTP 服务：`POST /qq/callback/{appid}` → Ed25519 验签（secret 重复至 32B 作 seed）→ op13 回签 / op0 事件进同一分发器。账号「连接方式」选 Webhook 即不起 WS，事件全部由回调投递。

## MCP 调试服务

`mcp_listen` + `mcp_token` 生效后，AI 客户端（Claude / Windsurf / Cline 等）配置：

```json
{
  "mcpServers": {
    "mashiro": {
      "url": "http://127.0.0.1:8465/mcp",
      "headers": { "Authorization": "Bearer <mcp_token>" }
    }
  }
}
```

工具清单：`plugin_list` `plugin_read` `plugin_write` `plugin_install` `plugin_reload` `plugin_set_enabled` `plugin_uninstall` `plugin_invoke` `logs_query` `timers_list` `kv_list` `kv_del` `send_message` `group_bot_state`。建议只绑回环地址。

## 插件开发

插件 = `plugin.json` + 入口文件，最小示例：

```js
// main.js（runtime: node）
const bot = require("mashiro");

bot.command("签到", async (ctx) => {
  const n = await ctx.kv.incr("score");
  await ctx.reply(`签到成功，积分 ${n}`);
});
```

```python
# main.py（runtime: python）
import mashiro

bot = mashiro.Bot()

@bot.command("签到")
def signin(ctx):
    n = ctx.kv.incr("score")
    ctx.reply(f"签到成功，积分 {n}")
```

- JS 插件只能 `require("mashiro")` 与 Node 内置模块；Python 插件只用标准库与 `mashiro`
- `permissions` 白名单强制校验，声明用到的能力即可
- JS 插件跑在各自 Worker 线程；Python 插件是同宿主内的独立模块（handler 独立线程）

完整规范（manifest 字段 / NDJSON 协议 / ctx / 全部插件 API / MessageEvent / 安装方式）见 [PLUGIN_SPEC.md](PLUGIN_SPEC.md)。

## 开源许可

本项目以 [GNU Affero General Public License v3.0 (AGPL-3.0)](LICENSE) 开源。

要点：可自由使用、修改、再分发；分发修改版或**通过网络向用户提供修改版服务**时，须以相同许可证公开对应源码。详见 LICENSE 全文。
