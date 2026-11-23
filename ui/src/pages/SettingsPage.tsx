import { useCallback, useEffect, useState } from "react";
import { App, AutoComplete, Button, Input, InputNumber, Modal, Segmented, Tooltip } from "antd";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { Icon } from "../components/Icons";

const KEYS = [
  "node_path",
  "python_path",
  "webhook_listen",
  "webhook_tls_cert",
  "webhook_tls_key",
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
  { key: "network", icon: "globe", label: "网络接入", desc: "Webhook 回调入口 · TLS 证书 · WS 事件中转" },
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
  // PEM 手动粘贴弹窗：kind = 在填证书还是私钥
  const [pemKind, setPemKind] = useState<"cert" | "key" | null>(null);
  const [pemText, setPemText] = useState("");
  const [pemSaving, setPemSaving] = useState(false);

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

  // 文件选择器填路径（证书/私钥不手打）
  const pickFile = async (key: string, exts: string[]) => {
    try {
      const p = await open({
        multiple: false,
        filters: [{ name: "PEM 证书", extensions: exts }],
      });
      if (typeof p === "string") set(key, p);
    } catch (e) {
      message.error(String(e));
    }
  };

  const openPem = (kind: "cert" | "key") => {
    setPemKind(kind);
    setPemText("");
  };

  const savePem = async () => {
    if (!pemKind) return;
    const content = pemText.trim();
    if (!content.includes("-----BEGIN")) {
      message.error("不是有效的 PEM 内容（应含 -----BEGIN …-----）");
      return;
    }
    setPemSaving(true);
    try {
      const path = await api.writePemFile(pemKind, content);
      set(pemKind === "cert" ? "webhook_tls_cert" : "webhook_tls_key", path);
      setPemKind(null);
      message.success("已写入并填入路径");
    } catch (e) {
      message.error(String(e));
    } finally {
      setPemSaving(false);
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
                {/* ── Webhook 回调服务 ── */}
                <div className="set-group">
                  <div className="set-group-title">Webhook 回调服务</div>
                  <div className="set-field">
                    <div className="set-field-label">监听地址</div>
                    <Input
                      placeholder="留空 = 关；如 0.0.0.0:8443"
                      value={vals.webhook_listen}
                      onChange={(e) => set("webhook_listen", e.target.value)}
                    />
                    <div className="set-hint">
                      回调地址 <code>POST &lt;监听&gt;/qq/callback/&#123;appid&#125;</code>。
                      官方要求回调必须 HTTPS，且只接受 80 / 443 / 8080 / 8443 端口。
                      改动需重启应用生效。
                    </div>
                  </div>
                  <div className="set-field">
                    <div className="set-field-label">TLS 证书 (PEM)</div>
                    <div className="set-field-row">
                      <Input readOnly placeholder="未配置" value={vals.webhook_tls_cert} />
                      <Button
                        onClick={() => pickFile("webhook_tls_cert", ["pem", "crt", "cer"])}
                      >
                        选择文件
                      </Button>
                      <Button onClick={() => openPem("cert")}>粘贴内容</Button>
                      {vals.webhook_tls_cert?.trim() && (
                        <Button onClick={() => set("webhook_tls_cert", "")}>清除</Button>
                      )}
                    </div>
                  </div>
                  <div className="set-field">
                    <div className="set-field-label">TLS 私钥 (PEM)</div>
                    <div className="set-field-row">
                      <Input readOnly placeholder="未配置" value={vals.webhook_tls_key} />
                      <Button onClick={() => pickFile("webhook_tls_key", ["pem", "key"])}>
                        选择文件
                      </Button>
                      <Button onClick={() => openPem("key")}>粘贴内容</Button>
                      {vals.webhook_tls_key?.trim() && (
                        <Button onClick={() => set("webhook_tls_key", "")}>清除</Button>
                      )}
                    </div>
                    <div className="set-hint">
                      都填 → 原生 <code>https://</code> 监听；都不填 → <code>http://</code>
                      （上游 nginx / caddy 反代终结 TLS，可自动申请 CA 证书）。
                      QQ 回调校验证书链，自签证书过不了——需 CA 签发证书。
                    </div>
                  </div>
                  <div className="set-field">
                    <div className="set-field-label">WS 事件中转</div>
                    <div className="set-hint" style={{ marginTop: 0 }}>
                      同一监听还提供 <code>ws(s)://&lt;地址&gt;/qq/ws</code>（全量）与{" "}
                      <code>/qq/ws/&lt;appid&gt;</code>（单账号）端点：把 webhook
                      事件帧原样推给 WS 客户端，协议仿真官方网关（Hello / Identify /
                      心跳）。对客户端暴露的地址与鉴权由中转部署层决定；
                      账号详情「中转网关地址」填中转服务给的 wss:// 即可。
                    </div>
                  </div>
                </div>

                {/* ── MCP ── */}
                <div className="set-group">
                  <div className="set-group-title">MCP 调试服务</div>
                  <div className="set-field">
                    <div className="set-field-label">监听地址</div>
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
                    触发 handler、查日志、发测试消息。改动保存后立即生效，无需重启；
                    建议只绑 127.0.0.1。
                  </div>
                    {vals.mcp_listen?.trim() !== "off" && (
                      <div style={{ position: "relative" }}>
                        <pre className="set-mcp-snippet mono">{`{
  "mcpServers": {
    "mashiro": {
      "url": "http://${vals.mcp_listen?.trim() || "127.0.0.1:8465"}/mcp",
      "headers": { "Authorization": "Bearer ${vals.mcp_token || "<token>"}" }
    }
  }
}`}</pre>
                        <Button
                          size="small"
                          style={{ position: "absolute", top: 6, right: 6 }}
                          onClick={() => {
                            navigator.clipboard
                              .writeText(`{
  "mcpServers": {
    "mashiro": {
      "url": "http://${vals.mcp_listen?.trim() || "127.0.0.1:8465"}/mcp",
      "headers": { "Authorization": "Bearer ${vals.mcp_token || "<token>"}" }
    }
  }
}`)
                              .then(() => message.success("已复制到剪贴板"))
                              .catch(() => message.error("复制失败"));
                          }}
                        >
                          复制
                        </Button>
                      </div>
                    )}
                  </div>
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

      {/* 手动粘贴 PEM：内容写盘到数据目录 tls/ 并回填路径 */}
      <Modal
        title={pemKind === "cert" ? "粘贴 TLS 证书 (PEM)" : "粘贴 TLS 私钥 (PEM)"}
        open={pemKind !== null}
        onOk={savePem}
        confirmLoading={pemSaving}
        onCancel={() => setPemKind(null)}
        okText="保存"
        cancelText="取消"
        width={560}
      >
        <Input.TextArea
          rows={12}
          className="mono"
          value={pemText}
          onChange={(e) => setPemText(e.target.value)}
          placeholder={
            pemKind === "cert"
              ? "-----BEGIN CERTIFICATE-----\n…（可含多张证书链）\n-----END CERTIFICATE-----"
              : "-----BEGIN PRIVATE KEY-----\n…\n-----END PRIVATE KEY-----"
          }
        />
        <div className="set-hint" style={{ marginTop: 8 }}>
          内容写入数据目录 <code>tls/webhook-{pemKind}.pem</code>
          并自动填入设置。私钥仅存本机，勿外发。
        </div>
      </Modal>
    </div>
  );
}
