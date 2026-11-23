import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { api } from "../api";
import appIcon from "../../../src-tauri/icons/128x128.png";

const REPO = "https://github.com/MoonSeaFishCat/shiina-mashiro";

const palette: [string, string][] = [
  ["窗口背景", "#FFF8EF"],
  ["内容区", "#F7EFE5"],
  ["主强调色", "#FF7F32"],
  ["强调浅底", "#FFF0DE"],
  ["主文字", "#263241"],
];

const stack: [string, string][] = [
  ["桌面壳", "Tauri 2.x"],
  ["核心", "Rust · Tokio · mashiro-core"],
  ["协议", "QQ 官方机器人 WebSocket / Webhook"],
  ["存储", "SQLite（rusqlite）· DPAPI 密钥加密"],
  ["前端", "React + TypeScript + Vite + antd"],
  ["插件", "Node.js Worker / Python 宿主 · NDJSON 协议"],
];

const changelog: { v: string; date: string; items: string[] }[] = [
  {
    v: "v0.5.1",
    date: "2026-11-23",
    items: [
      "UI 重构：首页仪表盘（问候横幅/机器人启停/事件流）、账号卡片化管理、右键菜单、素材拖拽上传、关于页时间轴、侧边栏账号下拉切换",
      "账号配置与订阅事件拆分独立弹窗，分组框化布局",
      "API 修复：频道私聊 /dms/{guild_id} 路径、分片上传 md5_10m 边界、is_wakeup 仅 C2C、SendOpts 补引用回复",
      "API 实测：48 方法真机验证（12MB 分片上传/禁言/踢人/互动应答全闭环）",
    ],
  },
  {
    v: "v0.5.0",
    date: "2026-11-23",
    items: [
      "事件全量解析：按钮回调/互动id/场景/操作者/入群方式/附件/@提及/回复引用/消息序号/机器人标记/@全员/事件子类型 全部直读，插件不再挖 raw",
      "修复定时器踢人不生效：timer 事件载荷顶层与 raw 双写兼容",
      "Agent 会话卡：模型每轮文字落定到流程对应位置（say 行），不再上下割裂",
      "修复误点/迟点停止按钮污染下一次生成的问题",
      "安全修复：工作区文件工具与插件载入路径校验，杜绝目录穿越",
    ],
  },
  {
    v: "v0.4.8",
    date: "2026-11-23",
    items: [
      "内置 Agent 可调全部框架工具（mcp_call）：查插件、模拟触发、看日志、管定时器/KV",
      "指令匹配兼容 / 前缀：/签到 和 签到 都能触发插件指令",
      "插件规范补全：完整可仿写示例 + SDK 接口速查表，Agent 生成的代码更准",
      "MCP logs_query 新增 plugin 参数按插件过滤日志",
    ],
  },
  {
    v: "v0.4.6",
    date: "2026-11-23",
    items: [
      "每指令触发权限：插件页逐条指令可设 停用/所有人/管理员/仅群主，改即生效",
      "MCP 监听配置免重启：改地址/换 token/关闭即存即生效，配置块一键复制",
      "access_token 获取失败时显示 QQ 平台返回的真实错误原因",
    ],
  },
  {
    v: "v0.4.5",
    date: "2026-11-23",
    items: [
      "插件可视化配置：plugin.json 声明 config_schema，插件页自动渲染 14 种组件（文本/密钥/数字/滑块/开关/单选/多选/标签/路径/文件/颜色/日期/时间）",
      "Agent 改已有插件：载入工作区直接改真实文件，装验即热重载",
      "多文件插件支持：JS require / Python import 辅助文件，子目录也行",
      "修复正式包插件宿主全部起不来的致命问题（资源目录拍平、ESM 误判、SDK 未打包）",
      "Agent 生成防卡死：流空闲超时判死 + 停止按钮 + 增量持久化 + 装验三连败保险丝",
      "会话卡片 markdown 渲染；read_file 上限放宽防误报文件截断",
    ],
  },
  {
    v: "v0.4.4",
    date: "2026-11-23",
    items: [
      "修复 Agent 生成卡死：模型接口流式连接假活不再永久转圈（空闲超时自动判死）",
      "Agent 新增停止按钮：生成中随时掐断，已产出文件保留",
      "产出文件面板改左竖排文件列表，文件多了不再挤",
    ],
  },
  {
    v: "v0.4.3",
    date: "2026-11-23",
    items: [
      "Agent 会话持久化：切页/重启自动恢复最近会话，顶部下拉可切换、删除历史会话",
      "Agent 产出文件实时上屏：模型每写一个文件右侧立刻可见，不用等整轮跑完",
      "生成中需求确认弹窗可找回，后台生成进度切回来自动续接",
      "修复生成中途报错丢失已产出文件、重启后安装接口失效等问题",
    ],
  },
  {
    v: "v0.4.2",
    date: "2026-11-23",
    items: [
      "更新弹窗：更新日志剥掉 markdown 符号渲染为干净文本，弹窗加宽",
      "侧栏更新入口改版式：版本号居左、检查更新居右，发现新版出 new 标记",
      "macOS 更新链路：universal .app 手动打包签名，darwin 双架构键补齐",
    ],
  },
  {
    v: "v0.4.0",
    date: "2026-11-23",
    items: [
      "在线更新：侧栏检查按钮 + 启动/每 30 分钟静默巡检，弹窗看更新日志和下载进度，装完一键重启",
      "更新托管走 Gitee/GitHub 双端点带签名校验，国内直连，三平台全支持",
      "Agent 生成输出改流式：逐字实时渲染不再干等",
      "Agent 新增需求确认弹窗：模型拿不准时弹窗点选项代替打字回答",
      "Agent 工具调用放开无限轮次，配上下文自动瘦身与重复调用保险丝",
      "SDK 新增 bot.log 调试日志接口（JS/Python 免事件上下文）",
      "侧栏版本号修正为读取真实版本",
    ],
  },
  {
    v: "v0.3.0",
    date: "2026-11-23",
    items: [
      "插件身份认证：load 时框架下发一次性 token，宿主回帧盖章校验，伪造帧一律匿名拒绝",
      "Node 插件 Worker 输出流与协议管道隔离：插件写 stdout 不再污染协议",
      "JS 插件加载等待 Worker 就绪握手：入口语法错误不再误报安装成功",
      "Python 插件入口在独立线程加载：顶层代码阻塞不再拖死整个宿主",
      "宿主进程崩溃自动重建并重放全部已装插件",
      "事件批投递：一条消息命中多个插件时单帧扇出，事件只序列化一次",
      "自动撤回改时间轮（120 槽 O(1) 常驻任务，上限 110 秒）",
      "网关心跳看门狗：僵尸连接自动重连；断线退避在长会话后归零",
      "插件 API 并发调度：单个插件的慢请求不再阻塞全部插件",
      "安全：/qq/ws 可配 ws_token 鉴权、定时任务只能删自己的、插件 id 防路径穿越、MCP token 定长比较",
      "修复：分片上传 md5_10m 常量、token 过期下溢 panic、两处 UTF-8 截断 panic、Python 宿主中文路径乱码、卸载残留定时任务、event:GROUP_MESSAGE_CREATE 漏收 @ 消息",
    ],
  },
  {
    v: "v0.2.0",
    date: "2026-11-22",
    items: [
      "Webhook 原生 HTTPS：PEM 证书/私钥（文件选择或粘贴落盘），也支持反代终结 TLS",
      "新增 /qq/ws 事件中转端点：webhook→websocket，协议仿真官方网关，key 鉴权 + 单账号过滤",
      "账号「自定义网关」：直连自建/第三方 webhook→websocket 中转，跳过官方握手",
      "QQ API v2 补全 30+ 端点：群管（成员/黑名单/批量踢/禁言/加群审批/策略）、交互应答、流式消息、本地分片上传、菜单/面板、msg_type 8 卡片",
      "插件 SDK（JS/Python）同步暴露新能力；默认订阅补群成员与互动事件",
      "修复 webhook op13 回签算法（event_ts+plain_token）、C2C_MSG_* 事件名解析",
    ],
  },
  {
    v: "v0.1.0",
    date: "2026-11-22",
    items: [
      "首个公开版本：多账号托管，WS / Webhook 双模式，沙盒与 intents 自定义订阅",
      "插件系统：Node.js / Python 双宿主、权限白名单、账号授权门、KV 存储",
      "Agent 一句话生成插件（OpenAI / Anthropic 工具调用循环）",
      "MCP 调试服务：AI 客户端直连管理插件与日志",
      "扫码一键登录、聊天/素材/任务/日志页面",
      "三平台 CI 发布（Windows NSIS / macOS universal dmg / Linux deb+AppImage）",
    ],
  },
];

export default function AboutPage() {
  const [ver, setVer] = useState("");
  const [expand, setExpand] = useState(false);
  useEffect(() => {
    getVersion().then(setVer).catch(() => {});
  }, []);

  const shown = expand ? changelog : changelog.slice(0, 5);

  return (
    <div className="about-page">
      <h2 className="page-title">关于</h2>

      {/* 品牌横幅 */}
      <div className="about-hero">
        <img className="about-icon" src={appIcon} alt="" draggable={false} />
        <div className="about-id">
          <div className="about-name-row">
            <span className="about-name">椎名真白</span>
            <span className="about-pill">v{ver || "…"}</span>
            <span className="about-pill lic">AGPL-3.0</span>
          </div>
          <div className="about-en">Shiina Mashiro</div>
          <div className="about-slogan">《纯白、无口、但群里每句话都记得》</div>
        </div>
        <button className="about-gh" onClick={() => api.openPath(REPO)}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
            <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.55v-2.17c-3.2.7-3.87-1.36-3.87-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.76 2.7 1.25 3.35.96.1-.75.4-1.25.72-1.54-2.55-.29-5.24-1.28-5.24-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.51-1.46.11-3.04 0 0 .97-.31 3.18 1.18a11.1 11.1 0 0 1 5.78 0c2.21-1.49 3.18-1.18 3.18-1.18.62 1.58.23 2.75.11 3.04.74.81 1.19 1.83 1.19 3.09 0 4.41-2.7 5.38-5.27 5.67.41.36.78 1.05.78 2.13v3.16c0 .3.2.67.8.55A11.51 11.51 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z" />
          </svg>
          GitHub
        </button>
      </div>

      {/* 信息三卡 */}
      <div className="about-grid">
        <div className="card set-card">
          <div className="set-card-title">定位</div>
          <div className="set-hint" style={{ fontSize: 12.5, color: "#888", lineHeight: 1.8 }}>
            纯白铃铛的 Rust + Tauri 重写瘦身版。框架只做管道：收消息 →
            指令前缀/事件订阅命中 → 投递插件 → 发消息。抽奖、商店、AI、签到
            等一切业务都属于插件，后续由 Agent 一句话生成。
          </div>
        </div>
        <div className="card set-card">
          <div className="set-card-title">技术栈</div>
          {stack.map(([k, v]) => (
            <div key={k} className="about-stack-row">
              <span>{k}</span>
              <b>{v}</b>
            </div>
          ))}
        </div>
        <div className="card set-card">
          <div className="set-card-title">主题配色</div>
          <div className="about-swatches">
            {palette.map(([name, c]) => (
              <div key={c} className="about-swatch">
                <div
                  className="sw-color"
                  style={{ background: c }}
                  title={c}
                />
                <div className="sw-name">{name}</div>
                <div className="sw-hex">{c}</div>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* 更新日志：时间轴 */}
      <div className="card set-card" style={{ marginTop: 12 }}>
        <div className="set-card-title">更新日志</div>
        <div className="chg-timeline">
          {shown.map((c, i) => (
            <div key={c.v} className={`chg-item${i === 0 ? " latest" : ""}`}>
              <div className="chg-node" />
              <div className="chg-head">
                <b className="chg-v">{c.v}</b>
                <span className="chg-d">{c.date}</span>
                {i === 0 && <span className="chg-tag">最新</span>}
              </div>
              <ul className="chg-list">
                {c.items.map((it) => (
                  <li key={it}>{it}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        {changelog.length > 5 && (
          <button className="chg-more" onClick={() => setExpand((e) => !e)}>
            {expand ? "收起" : `展开全部 ${changelog.length} 个版本`}
          </button>
        )}
      </div>

      <div className="about-foot">
        椎名真白 · AGPL-3.0-only · 基于 QQ 官方 Bot API v2 ·
        <span className="link" onClick={() => api.openPath(REPO)}>
          github.com/MoonSeaFishCat/shiina-mashiro
        </span>
      </div>
    </div>
  );
}
