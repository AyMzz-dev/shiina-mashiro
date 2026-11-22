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
      </div>
    </div>
  );
}
