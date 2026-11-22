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
    v: "未发布",
    date: "main",
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
  return (
    <div>
      <h2 className="page-title">关于</h2>
      <div className="card about-hero">
        <div className="about-logo">白</div>
        <div>
          <div className="about-name">椎名真白</div>
          <div className="about-en">Shiina Mashiro · v0.1.0</div>
          <div className="about-slogan">《纯白、无口、但群里每句话都记得》</div>
        </div>
      </div>

      <div className="set-grid" style={{ marginTop: 12 }}>
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
          <div className="set-card-title">主题配色（延续纯白铃铛）</div>
          <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
            {palette.map(([name, c]) => (
              <div key={c} style={{ textAlign: "center" }}>
                <div
                  style={{
                    width: 52,
                    height: 32,
                    borderRadius: 6,
                    background: c,
                    border: "1px solid var(--border)",
                  }}
                />
                <div
                  style={{ fontSize: 11, color: "var(--text-sub)", marginTop: 4 }}
                >
                  {name}
                  <br />
                  {c}
                </div>
              </div>
            ))}
          </div>
        </div>
        <div className="card set-card">
          <div className="set-card-title">更新日志</div>
          {changelog.map((c) => (
            <div key={c.v} className="chg-block">
              <div className="chg-ver">
                <b>{c.v}</b>
                <span className="d">{c.date}</span>
              </div>
              <ul className="chg-list">
                {c.items.map((it) => (
                  <li key={it}>{it}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
