import { useCallback, useEffect, useState } from "react";
import { App, AutoComplete, Button, Input, InputNumber, Segmented, Tooltip } from "antd";
import { api } from "../api";
import { Icon } from "../components/Icons";

const KEYS = [
  "node_path",
  "python_path",
  "webhook_listen",
  "mcp_listen",
  "mcp_token",
  "log_keep_days",
  "agent_provider",
  "agent_base",
  "agent_key",
  "agent_model",
] as const;

const PROVIDER_HINT: Record<string, { base: string; model: string }> = {
  openai: { base: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  anthropic: { base: "https://api.anthropic.com", model: "claude-sonnet-4-5" },
};

const CATS = [
  { key: "runtime", icon: "plugin", label: "运行时", desc: "JS / Python 插件各自的宿主解释器" },
  { key: "agent", icon: "spark", label: "Agent 模型", desc: "「生成」页用的大模型接入配置" },
  { key: "network", icon: "globe", label: "网络接入", desc: "Webhook 模式账号的事件入口" },
  { key: "logs", icon: "logs", label: "日志", desc: "日志保留策略与手动清理" },
  { key: "storage", icon: "db", label: "存储", desc: "全部数据的落地位置" },
];

export default function SettingsPage() {
  const { message } = App.useApp();
  const [cat, setCat] = useState("runtime");
  const [vals, setVals] = useState<Record<string, string>>({});
  const [dataDir, setDataDir] = useState("");
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [checks, setChecks] = useState<Record<string, string>>({});
  const [agentTesting, setAgentTesting] = useState(false);
  const [modelOpts, setModelOpts] = useState<{ value: string }[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);

  useEffect(() => {
    Promise.all(KEYS.map((k) => api.getSetting(k).then((v) => [k, v] as const)))
      .then((pairs) => setVals(Object.fromEntries(pairs)))
      .catch(() => {});
    api.dataDir().then(setDataDir).catch(() => {});
  }, []);

  const set = (k: string, v: string) => {
    setVals((s) => ({ ...s, [k]: v }));
    setDirty(true);
  };

  const save = async () => {
    setSaving(true);
    try {
      for (const k of KEYS) await api.setSetting(k, vals[k] ?? "");
      setDirty(false);
      message.success("已保存");
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const check = async (key: string, fallback: string) => {
    const path = vals[key]?.trim() || fallback;
    setChecks((s) => ({ ...s, [key]: "检测中…" }));
    try {
      const v = await api.checkProgram(path);
      setChecks((s) => ({ ...s, [key]: `✓ ${v}` }));
    } catch (e) {
      setChecks((s) => ({ ...s, [key]: `✗ ${e}` }));
    }
  };

  const fetchModels = async () => {
    setFetchingModels(true);
    try {
      const ids = await api.agentModels(
        vals.agent_provider || "openai",
        vals.agent_base ?? "",
        vals.agent_key ?? "",
      );
      setModelOpts(ids.map((id) => ({ value: id })));
      if (ids.length) {
        message.success(`获取到 ${ids.length} 个模型`);
      } else {
        message.warning("接口返回空列表");
      }
    } catch (e) {
      message.error(`获取失败：${e}`);
    } finally {
      setFetchingModels(false);
    }
  };

  const testAgent = async () => {
    setAgentTesting(true);
    setChecks((s) => ({ ...s, agent: "测试中…" }));
    try {
      const v = await api.agentTest(
        vals.agent_provider || "openai",
        vals.agent_base ?? "",
        vals.agent_key ?? "",
        vals.agent_model ?? "",
      );
      setChecks((s) => ({ ...s, agent: `✓ 连通（${v.trim() || "已响应"}）` }));
    } catch (e) {
      setChecks((s) => ({ ...s, agent: `✗ ${e}` }));
    } finally {
      setAgentTesting(false);
    }
  };

  const purge = async () => {
    try {
      const n = await api.purgeLogs();
      message.success(`已清理 ${n} 条过期日志`);
    } catch (e) {
      message.error(String(e));
    }
  };

  const openDir = useCallback(() => {
    if (dataDir) api.openPath(dataDir).catch((e) => message.error(String(e)));
  }, [dataDir, message]);

  const cur = CATS.find((c) => c.key === cat)!;

  const checkLine = (k: string) =>
    checks[k] && (
      <div className={`set-check ${checks[k].startsWith("✓") ? "ok" : "bad"}`}>
        {checks[k]}
      </div>
    );

  return (
    <div>
      <h2 className="page-title">设置</h2>
      <div className="set-wrap">
        {/* 顶部：标签页切换 */}
        <div className="set-tabs">
          {CATS.map((c) => (
            <button
              key={c.key}
              className={`set-tab${cat === c.key ? " on" : ""}`}
              onClick={() => setCat(c.key)}
            >
              <Icon name={c.icon} size={14} />
              {c.label}
            </button>
          ))}
        </div>

        {/* 当前分类面板 */}
        <div className="set-body">
          <div className="card set-card">
            <div className="set-card-head">
              <span className="set-card-title">{cur.label}</span>
              <span className="set-card-desc">{cur.desc}</span>
            </div>

            {cat === "runtime" && (
              <>
                <div className="set-field">
                  <div className="set-field-label">Node.js 解释器</div>
                  <div className="set-field-row">
                    <Input
                      placeholder="留空 = 按 PATH 找 node"
                      value={vals.node_path}
                      onChange={(e) => set("node_path", e.target.value)}
                    />
                    <Button onClick={() => check("node_path", "node")}>检测</Button>
                  </div>
                  {checkLine("node_path")}
                </div>
                <div className="set-field">
                  <div className="set-field-label">Python 解释器</div>
                  <div className="set-field-row">
                    <Input
                      placeholder="留空 = 按 PATH 找 python"
                      value={vals.python_path}
                      onChange={(e) => set("python_path", e.target.value)}
                    />
                    <Button onClick={() => check("python_path", "python")}>检测</Button>
                  </div>
                  {checkLine("python_path")}
                  <div className="set-hint">
                    JS/Python 插件各自由独立宿主进程执行；路径改动对下次拉起的宿主生效。
                  </div>
                </div>
              </>
            )}

            {cat === "agent" && (
              <>
                <div className="set-field">
                  <div className="set-field-label">接口协议</div>
                  <Segmented
                    value={vals.agent_provider || "openai"}
                    onChange={(v) => set("agent_provider", v as string)}
                    options={[
                      { label: "OpenAI 兼容", value: "openai" },
                      { label: "Anthropic", value: "anthropic" },
                    ]}
                  />
                </div>
                <div className="set-field">
                  <div className="set-field-label">API Base</div>
                  <Input
                    placeholder={PROVIDER_HINT[vals.agent_provider || "openai"]?.base}
                    value={vals.agent_base}
                    onChange={(e) => set("agent_base", e.target.value)}
                  />
                  <div className="set-hint">
                    OpenAI 兼容填到 <code>/v1</code> 或完整 <code>…/chat/completions</code>
                    均可；Anthropic 自动走 <code>/v1/messages</code>。
                  </div>
                </div>
                <div className="set-field">
                  <div className="set-field-label">API Key</div>
                  <Input.Password
                    placeholder="sk-…"
                    value={vals.agent_key}
                    onChange={(e) => set("agent_key", e.target.value)}
                  />
                </div>
                <div className="set-field">
                  <div className="set-field-label">模型</div>
                  <div className="set-field-row">
                    <AutoComplete
                      options={modelOpts}
                      value={vals.agent_model}
                      onChange={(v) => set("agent_model", v)}
                      placeholder={PROVIDER_HINT[vals.agent_provider || "openai"]?.model}
                      style={{ flex: 1 }}
                      filterOption={(input, opt) =>
                        (opt?.value ?? "").toLowerCase().includes(input.toLowerCase())
                      }
                    />
                    <Button loading={fetchingModels} onClick={fetchModels}>
                      获取列表
                    </Button>
                    <Button loading={agentTesting} onClick={testAgent}>
                      测试连接
                    </Button>
                  </div>
                  {checkLine("agent")}
                </div>
              </>
            )}

            {cat === "network" && (
              <>
                <div className="set-field">
                  <div className="set-field-label">Webhook 监听</div>
                  <Input
                    placeholder="留空 = 关；如 0.0.0.0:8464"
                    value={vals.webhook_listen}
                    onChange={(e) => set("webhook_listen", e.target.value)}
                  />
                  <div className="set-hint">
                    启用后回调地址为{" "}
                    <code>POST http://&lt;监听&gt;/qq/callback/&#123;appid&#125;</code>
                    ，供 QQ 平台把事件推给 Webhook 模式账号；改动需重启应用生效。
                  </div>
                </div>
                <div className="set-field">
                  <div className="set-field-label">MCP 调试服务</div>
                  <Input
                    placeholder="留空 = 自动 127.0.0.1:8465；off = 关"
                    value={vals.mcp_listen}
                    onChange={(e) => set("mcp_listen", e.target.value)}
                  />
                  <div className="set-field-row" style={{ marginTop: 8 }}>
                    <Input.Password
                      placeholder="留空 = 启动时自动生成"
                      value={vals.mcp_token}
                      onChange={(e) => set("mcp_token", e.target.value)}
                    />
                    <Button
                      onClick={() =>
                        set(
                          "mcp_token",
                          Array.from(crypto.getRandomValues(new Uint8Array(24)))
                            .map((b) => b.toString(16).padStart(2, "0"))
                            .join(""),
                        )
                      }
                    >
                      生成
                    </Button>
                  </div>
                  <div className="set-hint">
                    开箱即用：监听留空自动绑 <code>127.0.0.1:8465</code>、token 自动生成，
                    填 <code>off</code> 关闭。AI 客户端以{" "}
                    <code>Authorization: Bearer &lt;token&gt;</code> 连{" "}
                    <code>POST http://&lt;监听&gt;/mcp</code>，可直接读写/热重载插件、
                    触发 handler、查日志、发测试消息。改动需重启应用生效；建议只绑
                    127.0.0.1。
                  </div>
                  {vals.mcp_listen?.trim() !== "off" && (
                    <pre className="set-mcp-snippet mono">{`{
  "mcpServers": {
    "mashiro": {
      "url": "http://${vals.mcp_listen?.trim() || "127.0.0.1:8465"}/mcp",
      "headers": { "Authorization": "Bearer ${vals.mcp_token || "<token>"}" }
    }
  }
}`}</pre>
                  )}
                </div>
              </>
            )}

            {cat === "logs" && (
              <>
                <div className="set-field">
                  <div className="set-field-label">保留天数</div>
                  <div className="set-field-row">
                    <InputNumber
                      min={1}
                      max={3650}
                      value={Number(vals.log_keep_days) || 30}
                      onChange={(v) => set("log_keep_days", String(v ?? 30))}
                      style={{ width: 110 }}
                    />
                    <span className="set-hint-inline">天（每天自动清理过期日志）</span>
                  </div>
                </div>
                <div className="set-field">
                  <div className="set-field-label">手动清理</div>
                  <Button onClick={purge}>立即清理过期日志</Button>
                </div>
              </>
            )}

            {cat === "storage" && (
              <div className="set-field">
                <div className="set-field-label">数据目录</div>
                <div className="set-field-row">
                  <Tooltip title={dataDir}>
                    <span className="set-path">{dataDir || "…"}</span>
                  </Tooltip>
                  <Button onClick={openDir} disabled={!dataDir}>
                    打开
                  </Button>
                </div>
                <div className="set-hint">
                  账号、插件、日志、KV、定时任务全部存于该目录：mashiro.db（SQLite）
                  + plugins/（插件落地）。密钥按当前 Windows 用户加密存储。
                </div>
              </div>
            )}
          </div>

          <div className="set-savebar">
            <Button type="primary" loading={saving} disabled={!dirty} onClick={save}>
              保存设置
            </Button>
            {dirty && <span className="set-dirty">有未保存的修改</span>}
          </div>
        </div>
      </div>
    </div>
  );
}
