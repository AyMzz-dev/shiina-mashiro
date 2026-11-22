import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  App,
  Button,
  Checkbox,
  Empty,
  Input,
  InputNumber,
  Modal,
  Radio,
  Select,
  Switch,
  Tabs,
  Tag,
  Tooltip,
} from "antd";
import dayjs from "dayjs";
import { api, type Account, type Conversation, type GroupBotState, type LogRow } from "../api";
import { Icon } from "../components/Icons";

const KIND_NAME: Record<string, string> = {
  Group: "群",
  Guild: "频道",
  C2C: "私聊",
  GuildDm: "频道私聊",
};
const KIND_NUM: Record<string, number> = { Group: 0, Guild: 1, C2C: 2, GuildDm: 3 };
const RECV_KINDS = Object.keys(KIND_NAME);

const short = (s: string, n = 8) => (s.length > n ? s.slice(0, n) + "…" : s);
const rel = (ts: number) => {
  const d = dayjs(ts);
  return d.isSame(dayjs(), "day") ? d.format("HH:mm") : d.format("MM-DD HH:mm");
};

// ---------- 内联键盘编辑器 ----------

interface KbButton {
  label: string;
  visited: string;
  style: number; // 0 灰 / 1 蓝
  actionType: number; // 0 跳转 / 1 回调 / 2 指令
  data: string;
  permType: number; // 0 指定用户 / 1 仅管理 / 2 所有人 / 3 指定身份组
  permIds: string;
  reply: boolean;
  enter: boolean;
}

const newBtn = (): KbButton => ({
  label: "按钮",
  visited: "已点击",
  style: 1,
  actionType: 2,
  data: "",
  permType: 2,
  permIds: "",
  reply: false,
  enter: true,
});

function buildKeyboard(rows: KbButton[][]) {
  return {
    content: {
      rows: rows.map((r) => ({
        buttons: r.map((b, i) => ({
          id: String(i + 1),
          render_data: { label: b.label, visited_label: b.visited, style: b.style },
          action: {
            type: b.actionType,
            permission: {
              type: b.permType,
              ...(b.permType === 0
                ? { specify_user_ids: b.permIds.split(/[,\s]+/).filter(Boolean) }
                : b.permType === 3
                  ? { specify_role_ids: b.permIds.split(/[,\s]+/).filter(Boolean) }
                  : {}),
            },
            data: b.data,
            reply: b.reply,
            enter: b.enter,
            anchor: 0,
            unsupport_tips: "客户端不支持",
          },
        })),
      })),
    },
  };
}

function KeyboardEditor({
  open,
  onClose,
  onApply,
}: {
  open: boolean;
  onClose: () => void;
  onApply: (kb: object) => void;
}) {
  const [rows, setRows] = useState<KbButton[][]>([[newBtn()]]);
  const [sel, setSel] = useState<[number, number]>([0, 0]);
  const btn = rows[sel[0]]?.[sel[1]];

  const upd = (patch: Partial<KbButton>) => {
    setRows((rs) =>
      rs.map((r, i) =>
        i === sel[0] ? r.map((b, j) => (j === sel[1] ? { ...b, ...patch } : b)) : r,
      ),
    );
  };
  const kb = buildKeyboard(rows);

  return (
    <Modal
      title="内联键盘编辑"
      open={open}
      onCancel={onClose}
      width={720}
      okText="附加到消息"
      onOk={() => {
        onApply(kb);
        onClose();
      }}
    >
      <div className="kb-wrap">
        <div className="kb-grid">
          {rows.map((r, i) => (
            <div className="kb-row" key={i}>
              {r.map((b, j) => (
                <button
                  key={j}
                  className={`kb-btn s${b.style}${sel[0] === i && sel[1] === j ? " on" : ""}`}
                  onClick={() => setSel([i, j])}
                >
                  {b.label}
                </button>
              ))}
              {r.length < 5 && (
                <Button
                  size="small"
                  type="dashed"
                  onClick={() =>
                    setRows((rs) => rs.map((x, k) => (k === i ? [...x, newBtn()] : x)))
                  }
                >
                  +
                </Button>
              )}
              <Button
                size="small"
                type="text"
                danger
                onClick={() => setRows((rs) => rs.filter((_, k) => k !== i))}
              >
                删行
              </Button>
            </div>
          ))}
          {rows.length < 5 && (
            <Button size="small" type="dashed" onClick={() => setRows((rs) => [...rs, [newBtn()]])}>
              + 添加一行
            </Button>
          )}
        </div>
        {btn && (
          <div className="kb-form">
            <div className="kb-line">
              <span>文字</span>
              <Input size="small" value={btn.label} onChange={(e) => upd({ label: e.target.value })} />
            </div>
            <div className="kb-line">
              <span>点击后</span>
              <Input size="small" value={btn.visited} onChange={(e) => upd({ visited: e.target.value })} />
            </div>
            <div className="kb-line">
              <span>样式</span>
              <Radio.Group
                size="small"
                value={btn.style}
                onChange={(e) => upd({ style: e.target.value })}
                options={[
                  { label: "灰", value: 0 },
                  { label: "蓝", value: 1 },
                ]}
                optionType="button"
              />
              <span>动作</span>
              <Select
                size="small"
                style={{ width: 90 }}
                value={btn.actionType}
                onChange={(v) => upd({ actionType: v })}
                options={[
                  { label: "跳转", value: 0 },
                  { label: "回调", value: 1 },
                  { label: "指令", value: 2 },
                ]}
              />
            </div>
            <div className="kb-line">
              <span>数据</span>
              <Input
                size="small"
                placeholder={btn.actionType === 0 ? "https://…" : btn.actionType === 2 ? "/指令 参数" : "回调数据"}
                value={btn.data}
                onChange={(e) => upd({ data: e.target.value })}
              />
            </div>
            <div className="kb-line">
              <span>权限</span>
              <Select
                size="small"
                style={{ width: 120 }}
                value={btn.permType}
                onChange={(v) => upd({ permType: v })}
                options={[
                  { label: "指定用户", value: 0 },
                  { label: "仅管理员", value: 1 },
                  { label: "所有人", value: 2 },
                  { label: "指定身份组", value: 3 },
                ]}
              />
              {(btn.permType === 0 || btn.permType === 3) && (
                <Input
                  size="small"
                  placeholder="id 逗号分隔"
                  value={btn.permIds}
                  onChange={(e) => upd({ permIds: e.target.value })}
                />
              )}
            </div>
            <div className="kb-line">
              <Checkbox checked={btn.reply} onChange={(e) => upd({ reply: e.target.checked })}>
                reply（引用消息）
              </Checkbox>
              <Checkbox checked={btn.enter} onChange={(e) => upd({ enter: e.target.checked })}>
                enter（自动发送）
              </Checkbox>
              <Button
                size="small"
                danger
                type="text"
                onClick={() => {
                  setRows((rs) =>
                    rs
                      .map((r, i) => (i === sel[0] ? r.filter((_, j) => j !== sel[1]) : r))
                      .filter((r) => r.length > 0),
                  );
                  setSel([0, 0]);
                }}
              >
                删除此按钮
              </Button>
            </div>
          </div>
        )}
      </div>
      <pre className="kb-json">{JSON.stringify(kb, null, 1)}</pre>
    </Modal>
  );
}

// ---------- 聊天室 ----------

export default function ChatPage() {
  const { message } = App.useApp();
  const [convs, setConvs] = useState<Conversation[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [cur, setCur] = useState<Conversation | null>(null);
  const [msgs, setMsgs] = useState<LogRow[]>([]);
  const [text, setText] = useState("");
  const [markdown, setMarkdown] = useState(false);
  const [kb, setKb] = useState<object | null>(null);
  const [kbOpen, setKbOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const [search, setSearch] = useState("");
  const [authOpen, setAuthOpen] = useState(false);
  const [authState, setAuthState] = useState<GroupBotState | null>(null);
  const [authErr, setAuthErr] = useState("");
  const bodyRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);

  const loadConvs = useCallback(() => {
    api.listConversations().then(setConvs).catch(() => {});
  }, []);
  useEffect(() => {
    api.listAccounts().then(setAccounts).catch(() => {});
    loadConvs();
    const t = setInterval(loadConvs, 4000);
    return () => clearInterval(t);
  }, [loadConvs]);

  const loadMsgs = useCallback(
    (c: Conversation, before = 0) => {
      api
        .chatHistory(c.appid, c.group_id, before, 100)
        .then((rows) => {
          const asc = [...rows].reverse();
          setMsgs((old) => (before > 0 ? [...asc, ...old] : asc));
          if (stickRef.current) {
            requestAnimationFrame(() => {
              bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
            });
          }
        })
        .catch((e) => message.error(String(e)));
    },
    [message],
  );

  useEffect(() => {
    if (!cur) return;
    stickRef.current = true;
    loadMsgs(cur);
    const t = setInterval(() => loadMsgs(cur), 2500);
    return () => clearInterval(t);
  }, [cur, loadMsgs]);

  const nameOf = (appid: number) => accounts.find((a) => a.appid === appid)?.name || String(appid);

  // 会话类型 + 被动回复 msg_id：取最近一条「接收」日志
  const lastRecv = useMemo(
    () => [...msgs].reverse().find((m) => RECV_KINDS.includes(m.kind)),
    [msgs],
  );
  const convKind = lastRecv?.kind ?? (cur && RECV_KINDS.includes(cur.kind) ? cur.kind : "Group");

  const filtered = convs.filter(
    (c) =>
      !search ||
      c.group_id.includes(search) ||
      c.last_content.includes(search) ||
      nameOf(c.appid).includes(search),
  );

  const send = async () => {
    if (!cur || !text.trim()) return;
    setSending(true);
    try {
      await api.sendChat({
        appid: cur.appid,
        openid: cur.group_id,
        kind: KIND_NUM[convKind] ?? 0,
        content: text.trim(),
        markdown,
        keyboard: kb ?? undefined,
      });
      setText("");
      setKb(null);
      stickRef.current = true;
      loadMsgs(cur);
    } catch (e) {
      message.error(String(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="chat-page">
      <div className="card conv-list">
        <Input
          size="small"
          placeholder="搜索会话"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          allowClear
        />
        <div className="conv-items">
          {filtered.length === 0 && <Empty description="暂无会话" image={Empty.PRESENTED_IMAGE_SIMPLE} />}
          {filtered.map((c) => (
            <div
              key={`${c.appid}:${c.group_id}`}
              className={`conv-item${cur?.appid === c.appid && cur?.group_id === c.group_id ? " on" : ""}`}
              onClick={() => setCur(c)}
            >
              <div className="conv-top">
                <Tag className={`conv-kind k-${c.kind}`}>{KIND_NAME[c.kind] ?? c.kind}</Tag>
                <b className="conv-id" title={c.group_id}>
                  {short(c.group_id, 12)}
                </b>
                <span className="conv-time">{rel(c.last_ts)}</span>
              </div>
              <div className="conv-prev">
                {c.last_content || <i>(空)</i>}
                <span className="conv-bot">{nameOf(c.appid)}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="card chat-main">
        {!cur ? (
          <Empty description="选择左侧会话开始聊天" style={{ margin: "auto" }} />
        ) : (
          <>
            <div className="chat-head">
              <Tag className={`conv-kind k-${convKind}`}>{KIND_NAME[convKind]}</Tag>
              <b className="mono">{short(cur.group_id, 24)}</b>
              <span className="chat-sub">{nameOf(cur.appid)} · {msgs.length} 条</span>
              <div className="spacer" />
              {convKind === "Group" && (
                <Tooltip title="群授权状态：主动发言 / 消息范围">
                  <Button
                    size="small"
                    onClick={async () => {
                      setAuthOpen(true);
                      setAuthState(null);
                      setAuthErr("");
                      try {
                        setAuthState(await api.groupBotState(cur.appid, cur.group_id));
                      } catch (e) {
                        setAuthErr(String(e));
                      }
                    }}
                  >
                    授权
                  </Button>
                </Tooltip>
              )}
            </div>
            <div
              className="chat-body"
              ref={bodyRef}
              onScroll={(e) => {
                const el = e.currentTarget;
                stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
              }}
            >
              {msgs.length >= 100 && (
                <div className="chat-more">
                  <Button size="small" type="link" onClick={() => loadMsgs(cur, msgs[0]?.id ?? 0)}>
                    加载更早的消息
                  </Button>
                </div>
              )}
              {msgs.map((m) =>
                m.kind === "Send" ? (
                  <div className="bub out" key={m.id}>
                    <div className="bub-body">{m.content}</div>
                    <div className="bub-meta">{rel(m.ts)}</div>
                  </div>
                ) : (
                  <div className="bub in" key={m.id}>
                    <div className="bub-head">
                      {short(m.user, 8)} · {rel(m.ts)}
                    </div>
                    <div className="bub-body">{m.content}</div>
                  </div>
                ),
              )}
            </div>
            <div className="chat-composer">
              {kb && (
                <div className="chat-kb-hint">
                  <Icon name="keyboard" size={13} /> 已附加内联键盘（{JSON.stringify((kb as { content?: { rows?: unknown[] } }).content?.rows?.length ?? 0)} 行）
                  <Button size="small" type="link" danger onClick={() => setKb(null)}>
                    移除
                  </Button>
                </div>
              )}
              <Input.TextArea
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder={markdown ? "Markdown 内容…" : "输入消息…（Ctrl+Enter 发送）"}
                autoSize={{ minRows: 2, maxRows: 6 }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && e.ctrlKey) send();
                }}
              />
              <div className="chat-bar">
                <Tooltip title="Markdown 模式（受账号 md 开关约束）">
                  <span className="chat-md">
                    <Switch size="small" checked={markdown} onChange={setMarkdown} /> MD
                  </span>
                </Tooltip>
                <Tooltip title="内联键盘">
                  <Button size="small" icon={<Icon name="keyboard" size={13} />} onClick={() => setKbOpen(true)}>
                    键盘
                  </Button>
                </Tooltip>
                <div className="spacer" />
                <Button type="primary" size="small" loading={sending} onClick={send} icon={<Icon name="send" size={13} />}>
                  发送
                </Button>
              </div>
            </div>
          </>
        )}
      </div>

      <KeyboardEditor open={kbOpen} onClose={() => setKbOpen(false)} onApply={setKb} />

      <Modal
        title="群授权状态"
        open={authOpen}
        onCancel={() => setAuthOpen(false)}
        footer={null}
        width={460}
      >
        {cur && (
          <div className="auth-panel">
            {authState ? (
              <>
                <div className="auth-row">
                  <span>主动在群聊内发言</span>
                  {authState.allow_proactive_msg ? (
                    <Tag color="green">已开启</Tag>
                  ) : (
                    <Tag color="red">未开启</Tag>
                  )}
                </div>
                <div className="auth-row">
                  <span>可获取的消息范围</span>
                  <Tag color={authState.recv_msg_setting === "all" ? "green" : "orange"}>
                    {authState.recv_msg_setting === "all"
                      ? "全部消息"
                      : authState.recv_msg_setting === "mention_and_context"
                        ? "@ + 上下文"
                        : "仅 @ 消息"}
                  </Tag>
                </div>
                <div className="auth-row">
                  <span>机器人群内角色</span>
                  <Tag>
                    {authState.member_role === "owner"
                      ? "群主"
                      : authState.member_role === "admin"
                        ? "管理员"
                        : "成员"}
                  </Tag>
                </div>
              </>
            ) : authErr ? (
              <div className="auth-err">
                状态查询不可用：{authErr}
                <br />
                <span>（接口对部分账号限白名单；仍可下发指引卡让群主开启）</span>
              </div>
            ) : (
              <div className="auth-err">查询中…</div>
            )}
            <div className="auth-tip">
              两项开关均需<b>群主在手机 QQ</b> 打开：群成员列表 →「机器人」分组 →
              点本机器人 → 资料页右上角 ⚙；或 群聊页 ☰ → 聊天信息 → 群机器人 → 管理。
              点击下方按钮把指引卡发到群里。
            </div>
            <Button
              type="primary"
              block
              onClick={async () => {
                try {
                  await api.sendAuthCard(cur.appid, cur.group_id);
                  message.success("指引卡已发送");
                  setAuthOpen(false);
                  loadMsgs(cur);
                } catch (e) {
                  message.error(String(e));
                }
              }}
            >
              发送授权指引卡到群
            </Button>
          </div>
        )}
      </Modal>
    </div>
  );
}
