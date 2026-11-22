import { useEffect, useRef, useState } from "react";
import { App, AutoComplete, Button, Input, Segmented, Tag, Tooltip } from "antd";
import { listen } from "@tauri-apps/api/event";
import { api, type AgentFile } from "../api";
import { Icon } from "../components/Icons";

type Turn =
  | { role: "user"; text: string }
  | {
      role: "agent";
      lines: string[];
      reply?: string;
      files?: AgentFile[];
      error?: string;
      done?: boolean;
    };

/** 对话式插件生成工作台：左聊右码，多轮改稿，满意后一键安装/热重载。 */
export default function AgentPage() {
  const { message } = App.useApp();
  const [session, setSession] = useState<string | null>(null);
  const [runtime, setRuntime] = useState("node");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [files, setFiles] = useState<AgentFile[]>([]);
  const [selFile, setSelFile] = useState("");
  const [installed, setInstalled] = useState<string | null>(null);
  const [model, setModel] = useState("");
  const [modelOpts, setModelOpts] = useState<{ value: string }[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);

  // 后端 agent_progress 事件 → 追加到最后一张进行中的 agent 卡
  useEffect(() => {
    const un = listen<{ session: string; line: string }>("agent_progress", (ev) => {
      setTurns((t) => {
        const last = t[t.length - 1];
        if (!last || last.role !== "agent" || last.done) return t;
        const copy = t.slice();
        copy[copy.length - 1] = { ...last, lines: [...last.lines, ev.payload.line] };
        return copy;
      });
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
  }, [turns]);

  // 模型选择 = agent_model 设置的即时入口（后端 ProviderCfg 从设置读）
  useEffect(() => {
    api.getSetting("agent_model").then(setModel).catch(() => {});
  }, []);

  const pickModel = (v: string) => {
    setModel(v);
    api.setSetting("agent_model", v).catch(() => {});
  };

  const fetchModels = async () => {
    setFetchingModels(true);
    try {
      const [provider, base, key] = await Promise.all([
        api.getSetting("agent_provider"),
        api.getSetting("agent_base"),
        api.getSetting("agent_key"),
      ]);
      const ids = await api.agentModels(provider || "openai", base, key);
      setModelOpts(ids.map((id) => ({ value: id })));
      if (ids.length) message.success(`获取到 ${ids.length} 个模型`);
      else message.warning("接口返回空列表");
    } catch (e) {
      message.error(`获取失败：${e}`);
    } finally {
      setFetchingModels(false);
    }
  };

  const patchLast = (fn: (x: Turn) => Turn) =>
    setTurns((t) => {
      const c = t.slice();
      c[c.length - 1] = fn(c[c.length - 1]);
      return c;
    });

  const send = async () => {
    const prompt = input.trim();
    if (!prompt || busy) return;
    setBusy(true);
    setInput("");
    setTurns((t) => [
      ...t,
      { role: "user", text: prompt },
      { role: "agent", lines: [] },
    ]);
    try {
      const res = await api.agentGenerate(session, runtime, prompt);
      setSession(res.session);
      setFiles(res.files);
      setSelFile((s) => (res.files.some((f) => f.path === s) ? s : res.files[0]?.path ?? ""));
      patchLast((x) => ({ ...x, reply: res.text, files: res.files, done: true }));
    } catch (e) {
      patchLast((x) => ({ ...x, error: String(e), done: true }));
    } finally {
      setBusy(false);
    }
  };

  const install = async () => {
    if (!session || busy) return;
    setBusy(true);
    try {
      const res = await api.agentInstall(session);
      setInstalled(res.id);
      message.success(`已安装并启用 ${res.name}`);
    } catch (e) {
      message.error(`安装失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    setSession(null);
    setTurns([]);
    setFiles([]);
    setSelFile("");
    setInstalled(null);
  };

  const cur = files.find((f) => f.path === selFile);
  const manifest = (() => {
    try {
      return JSON.parse(files.find((f) => f.path === "plugin.json")?.content ?? "");
    } catch {
      return null;
    }
  })();

  return (
    <div className="agent-page">
      <div className="agent-chat card">
        <div className="agent-log" ref={logRef}>
          {turns.length === 0 && (
            <div className="agent-empty">
              一句话描述插件需求，例如「群签到：每天首次发言 +10 分，连续签到翻倍」。
              <br />
              生成后可继续对话修改，满意后点右侧「安装并启用」，改完再点即热重载。
            </div>
          )}
          {turns.map((t, i) =>
            t.role === "user" ? (
              <div key={i} className="agent-turn user">
                <div className="bubble">{t.text}</div>
              </div>
            ) : (
              <div key={i} className="agent-turn bot">
                <div className="agent-card">
                  {t.lines.map((l, j) => (
                    <div key={j} className="line">
                      {l}
                    </div>
                  ))}
                  {t.reply && <div className="reply">{t.reply}</div>}
                  {t.files && (
                    <div className="line ok">
                      产出：{t.files.map((f) => f.path).join("、")}
                    </div>
                  )}
                  {t.error && <div className="line err">{t.error}</div>}
                  {!t.done && <span className="agent-spin" />}
                </div>
              </div>
            ),
          )}
        </div>
        <div className="agent-composer">
          <Input.TextArea
            className="composer-ta"
            variant="borderless"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onPressEnter={(e) => {
              if (!e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder={
              session
                ? "继续说怎么改，例如「签到指令改成打卡，积分存 kv」…"
                : "描述插件需求（Enter 发送 / Shift+Enter 换行）"
            }
            autoSize={{ minRows: 2, maxRows: 6 }}
          />
          <div className="composer-bar">
            <Segmented
              size="small"
              value={runtime}
              disabled={!!session}
              onChange={(v) => setRuntime(v as string)}
              options={[
                { label: "Node.js", value: "node" },
                { label: "Python", value: "python" },
              ]}
            />
            <AutoComplete
              size="small"
              className="composer-model"
              options={modelOpts}
              value={model}
              onChange={pickModel}
              placeholder="模型名"
              filterOption={(input, opt) =>
                (opt?.value ?? "").toLowerCase().includes(input.toLowerCase())
              }
            />
            <Tooltip title="从接口拉取模型列表（用设置页的 Base + Key）">
              <Button
                size="small"
                type="text"
                loading={fetchingModels}
                onClick={fetchModels}
                icon={<Icon name="restart" size={13} />}
              />
            </Tooltip>
            <div className="composer-spacer" />
            <Tooltip title="丢弃当前会话重新开始">
              <Button
                size="small"
                onClick={reset}
                disabled={!session && turns.length === 0}
              >
                新会话
              </Button>
            </Tooltip>
            <Button size="small" type="primary" loading={busy} onClick={send}>
              发送
            </Button>
          </div>
        </div>
      </div>

      <div className="agent-files card">
        <div className="agent-files-head">
          <span className="t">产出文件</span>
          {manifest?.name && (
            <Tag color="orange">
              {manifest.name} · {manifest.id}
            </Tag>
          )}
        </div>
        {files.length === 0 ? (
          <div className="agent-files-empty">
            生成后这里会显示 plugin.json 和入口文件
          </div>
        ) : (
          <>
            <div className="agent-file-tabs">
              {files.map((f) => (
                <button
                  key={f.path}
                  className={`ftab${f.path === selFile ? " on" : ""}`}
                  onClick={() => setSelFile(f.path)}
                >
                  {f.path}
                </button>
              ))}
            </div>
            <pre className="agent-code">{cur?.content}</pre>
            <div className="agent-files-foot">
              <Button type="primary" onClick={install} loading={busy} disabled={!session}>
                {installed ? "热重载" : "安装并启用"}
              </Button>
              {installed && (
                <span className="hint">已安装 {installed}，改稿后再点直接热重载</span>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
