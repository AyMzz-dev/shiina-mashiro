import { useEffect, useRef, useState } from "react";
import {
  App,
  AutoComplete,
  Button,
  Input,
  Modal,
  Popconfirm,
  Radio,
  Segmented,
  Select,
  Tag,
  Tooltip,
} from "antd";
import { listen } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, type AgentFile, type AgentSessionInfo, type AskQuestion } from "../api";
import { Icon } from "../components/Icons";

type Turn =
  | { role: "user"; text: string }
  | {
      role: "agent";
      lines: string[];
      stream?: string;
      reply?: string;
      files?: AgentFile[];
      error?: string;
      done?: boolean;
    };

/** 对话式插件生成工作台：左聊右码，多轮改稿，满意后一键安装/热重载。
 *  会话后端持久化：切页/重启回来自动恢复最近会话，可下拉切换历史会话。 */
export default function AgentPage() {
  const { message } = App.useApp();
  const [session, setSession] = useState<string | null>(null);
  const [sessionList, setSessionList] = useState<AgentSessionInfo[]>([]);
  const [pluginOpts, setPluginOpts] = useState<{ value: string; label: string }[]>([]);
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
  const [ask, setAsk] = useState<{ callId: string; questions: AskQuestion[] } | null>(null);
  const [askAns, setAskAns] = useState<string[]>([]);
  const [askCustom, setAskCustom] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);
  // 事件按会话过滤：监听器只挂一次，用 ref 拿"当前会话"避免闭包陈旧
  const sessionRef = useRef<string | null>(null);
  const setSid = (s: string | null) => {
    sessionRef.current = s;
    setSession(s);
  };
  const sameSession = (sid: string) => sessionRef.current === sid;

  // 打开提问弹窗（事件与恢复共用）
  const openAsk = (callId: string, questions: AskQuestion[]) => {
    setAsk({ callId, questions });
    setAskAns(questions.map((q) => q.options[0] ?? ""));
    setAskCustom(questions.map(() => ""));
  };

  const restoreSession = async (sid: string) => {
    try {
      const st = await api.agentSessionState(sid);
      setSid(sid);
      setRuntime(st.runtime);
      setFiles(st.files);
      setSelFile(st.files[0]?.path ?? "");
      setAsk(null);
      setTurns(
        st.turns.map((t) =>
          t.role === "user"
            ? { role: "user", text: t.text }
            : { role: "agent", lines: [], reply: t.text, done: true },
        ),
      );
      // busy 必须对齐后端态：在别的会话生成中切过来不能永远转圈
      setBusy(st.active);
      if (st.active) {
        // 后台还在生成：补一张进行中的卡接住流式事件
        setTurns((t) => [...t, { role: "agent", lines: [] }]);
      }
      // manifest.id 已在插件列表里 → 按钮显示「热重载」而不是「安装并启用」
      try {
        const mf = JSON.parse(st.files.find((f) => f.path === "plugin.json")?.content ?? "");
        if (mf?.id) {
          const plugins = await api.listPlugins();
          if (plugins.some((p) => p.dir.split(/[\\/]/).pop() === mf.id)) setInstalled(mf.id);
          else setInstalled(null);
        } else setInstalled(null);
      } catch {
        setInstalled(null);
      }
      const pa = await api.agentPendingAsk(sid);
      if (pa) openAsk(pa.call_id, pa.questions);
    } catch {
      setSessionList((l) => l.filter((s) => s.id !== sid));
    }
  };

  // 首次挂载：拉会话列表并恢复最近一个
  useEffect(() => {
    (async () => {
      try {
        const list = await api.agentSessions();
        setSessionList(list);
        if (list.length) await restoreSession(list[0].id);
        // 「改已有插件」入口数据源：插件 id 取目录名
        const plgs = await api.listPlugins().catch(() => []);
        setPluginOpts(
          plgs.map((p) => ({
            value: p.dir.split(/[\\/]/).filter(Boolean).pop() ?? p.uuid,
            label: p.name,
          })),
        );
      } catch {
        /* 后端还没起来就算了 */
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 后端事件 → 聊天卡 / 文件栏 / 提问弹窗（全部按 session 过滤）
  useEffect(() => {
    const un = listen<{ session: string; line: string }>("agent_progress", (ev) => {
      if (!sameSession(ev.payload.session)) return;
      setTurns((t) => {
        const last = t[t.length - 1];
        if (!last || last.role !== "agent" || last.done) return t;
        const copy = t.slice();
        // say: 开头的流程行 = 模型本轮的文字输出落定——顺手清掉底部 stream，文字归位到流程里
        copy[copy.length - 1] = {
          ...last,
          lines: [...last.lines, ev.payload.line],
          stream: ev.payload.line.startsWith("say:") ? "" : last.stream,
        };
        return copy;
      });
    });
    const unDelta = listen<{ session: string; delta: string }>("agent_delta", (ev) => {
      if (!sameSession(ev.payload.session)) return;
      setTurns((t) => {
        const last = t[t.length - 1];
        if (!last || last.role !== "agent" || last.done) return t;
        const copy = t.slice();
        copy[copy.length - 1] = { ...last, stream: (last.stream ?? "") + ev.payload.delta };
        return copy;
      });
    });
    const unAsk = listen<{ session: string; call_id: string; questions: AskQuestion[] }>(
      "agent_ask",
      (ev) => {
        if (!sameSession(ev.payload.session)) return;
        openAsk(ev.payload.call_id, ev.payload.questions);
      },
    );
    const unFiles = listen<{ session: string; files: AgentFile[] }>("agent_files", (ev) => {
      if (!sameSession(ev.payload.session)) return;
      setFiles(ev.payload.files);
      setSelFile((s) =>
        ev.payload.files.some((f) => f.path === s) ? s : ev.payload.files[0]?.path ?? "",
      );
    });
    const unDone = listen<{ session: string; text?: string; error?: string }>(
      "agent_done",
      (ev) => {
        // 任意会话结束都刷新列表（updated_at 变了）；只有当前会话才碰卡片
        api.agentSessions().then(setSessionList).catch(() => {});
        if (!sameSession(ev.payload.session)) return;
        setTurns((t) => {
          const last = t[t.length - 1];
          if (!last || last.role !== "agent" || last.done) return t;
          const copy = t.slice();
          copy[copy.length - 1] = ev.payload.error
            ? { ...last, error: ev.payload.error, done: true }
            : { ...last, reply: ev.payload.text ?? last.reply ?? "", done: true };
          return copy;
        });
        setBusy(false);
      },
    );
    return () => {
      un.then((f) => f());
      unDelta.then((f) => f());
      unAsk.then((f) => f());
      unFiles.then((f) => f());
      unDone.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    // 新会话由前端先定 sid：首个事件包到来前 sessionRef 就就位，过滤才不漏
    const sid = session ?? `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    if (!session) setSid(sid);
    setBusy(true);
    setInput("");
    setTurns((t) => [
      ...t,
      { role: "user", text: prompt },
      { role: "agent", lines: [] },
    ]);
    try {
      const res = await api.agentGenerate(sid, runtime, prompt);
      setSid(res.session);
      setFiles(res.files);
      setSelFile((s) => (res.files.some((f) => f.path === s) ? s : res.files[0]?.path ?? ""));
      patchLast((x) => ({ ...x, reply: res.text, files: res.files, done: true }));
      api.agentSessions().then(setSessionList).catch(() => {});
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

  const submitAsk = async (cancelled?: boolean) => {
    if (!ask) return;
    const answers = cancelled
      ? ask.questions.map(() => "用户取消了提问")
      : ask.questions.map((_, i) =>
          askAns[i] === "__custom" ? askCustom[i].trim() || "（未回答）" : askAns[i],
        );
    const callId = ask.callId;
    setAsk(null);
    await api.agentAnswer(callId, answers).catch(() => {});
  };

  const reset = () => {
    setSid(null);
    setTurns([]);
    setFiles([]);
    setSelFile("");
    setInstalled(null);
    setAsk(null);
  };

  // 载入已有插件到会话：文件灌进草稿，接着说怎么改即可
  const attachPlugin = async (pid: string) => {
    try {
      const r = await api.agentAttachPlugin(session, pid);
      setSid(r.session);
      setRuntime(r.runtime);
      setFiles(r.files);
      setSelFile(r.files.find((f) => f.path === "plugin.json")?.path ?? r.files[0]?.path ?? "");
      setInstalled(pid);
      setTurns((t) => [
        ...t,
        { role: "agent", lines: [`已载入工作区 ${pid}（${r.files.length} 个文件，直接改真文件），说怎么改吧`], done: true },
      ]);
      api.agentSessions().then(setSessionList).catch(() => {});
    } catch (e) {
      message.error(`载入失败：${e}`);
    }
  };

  const delSession = async () => {
    if (!session) return;
    try {
      await api.agentSessionDelete(session);
      setSessionList((l) => l.filter((s) => s.id !== session));
      message.success("会话已删除");
    } catch (e) {
      message.error(`删除失败：${e}`);
    }
    reset();
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
        {(sessionList.length > 0 || session || pluginOpts.length > 0) && (
          <div className="agent-session-bar">
            <Select
              size="small"
              className="agent-session-select"
              placeholder="选择历史会话"
              value={session}
              onChange={(v) => restoreSession(v)}
              options={sessionList.map((s) => ({
                value: s.id,
                label: `${s.title || "（无标题）"} · ${s.runtime}`,
              }))}
              showSearch
              optionFilterProp="label"
            />
            {pluginOpts.length > 0 && (
              <Select
                size="small"
                style={{ width: 150 }}
                placeholder="载入插件工作区…"
                value={null}
                onChange={attachPlugin}
                options={pluginOpts}
                showSearch
                optionFilterProp="label"
              />
            )}
            {session && (
              <Popconfirm
                title="删除这个会话？"
                description="对话记录和未安装的文件草稿一起清除"
                okText="删除"
                cancelText="取消"
                onConfirm={delSession}
              >
                <Button size="small" type="text" danger>
                  删除
                </Button>
              </Popconfirm>
            )}
          </div>
        )}
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
                  {t.lines.map((l, j) =>
                    l.startsWith("say:") ? (
                      <div key={j} className="reply md">
                        <ReactMarkdown remarkPlugins={[remarkGfm]}>
                          {l.slice(4)}
                        </ReactMarkdown>
                      </div>
                    ) : (
                      <div key={j} className="line">
                        {l}
                      </div>
                    ),
                  )}
                  {t.reply ? (
                    <div className="reply md">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{t.reply}</ReactMarkdown>
                    </div>
                  ) : (
                    t.stream && (
                      <div className="reply md">
                        <ReactMarkdown remarkPlugins={[remarkGfm]}>{t.stream}</ReactMarkdown>
                      </div>
                    )
                  )}
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
            {busy && session ? (
              <Tooltip title="停止当前生成（已产出文件保留）">
                <Button size="small" danger onClick={() => api.agentCancel(session)}>
                  停止
                </Button>
              </Tooltip>
            ) : null}
            <Button size="small" type="primary" loading={busy} onClick={send} disabled={busy}>
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
            <div className="agent-files-body">
              <div className="agent-file-list">
                {files.map((f) => (
                  <button
                    key={f.path}
                    title={f.path}
                    className={`fitem${f.path === selFile ? " on" : ""}`}
                    onClick={() => setSelFile(f.path)}
                  >
                    {f.path}
                  </button>
                ))}
              </div>
              <pre className="agent-code">{cur?.content}</pre>
            </div>
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

      <Modal
        open={!!ask}
        title="插件需求确认"
        okText="确认"
        cancelText="跳过"
        onOk={() => submitAsk()}
        onCancel={() => submitAsk(true)}
        maskClosable={false}
      >
        {ask?.questions.map((q, i) => (
          <div key={i} style={{ marginBottom: 14 }}>
            <div style={{ marginBottom: 6 }}>
              {i + 1}. {q.q}
            </div>
            <Radio.Group
              value={askAns[i]}
              onChange={(e) =>
                setAskAns((a) => {
                  const c = [...a];
                  c[i] = e.target.value;
                  return c;
                })
              }
              options={[
                ...q.options.map((o) => ({ label: o, value: o })),
                { label: "自定义…", value: "__custom" },
              ]}
            />
            {askAns[i] === "__custom" && (
              <Input
                size="small"
                style={{ marginTop: 6 }}
                placeholder="输入你的答案"
                value={askCustom[i]}
                onChange={(e) =>
                  setAskCustom((a) => {
                    const c = [...a];
                    c[i] = e.target.value;
                    return c;
                  })
                }
              />
            )}
          </div>
        ))}
      </Modal>
    </div>
  );
}
