import { useCallback, useEffect, useRef, useState } from "react";
import {
  App,
  Button,
  Checkbox,
  Drawer,
  Form,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Radio,
  Space,
  Tag,
} from "antd";
import QRCode from "qrcode";
import { api, type Account, type BotInfo } from "../api";
import { BotAvatar } from "../components/TitleBar";

const SECRET_MASK = "••••••";

/** 与旧框架一致的 12 个 intents 订阅项 */
const INTENTS: { name: string; mask: number }[] = [
  { name: "GUILDS(频道事件)", mask: 1 << 0 },
  { name: "GUILD_MEMBERS(成员加入)", mask: 1 << 1 },
  { name: "GUILD_MESSAGES(私域·无艾特)", mask: 1 << 9 },
  { name: "GUILD_MESSAGE_REACTIONS(添加表情)", mask: 1 << 10 },
  { name: "DIRECT_MESSAGE(频道私聊事件)", mask: 1 << 12 },
  { name: "GROUP_MEMBER(群成员进出/申请)", mask: 1 << 24 },
  { name: "GROUP_AND_C2C_EVENT(群聊和私聊)", mask: 1 << 25 },
  { name: "INTERACTION(互动事件)", mask: 1 << 26 },
  { name: "MESSAGE_AUDIT(审核事件)", mask: 1 << 27 },
  { name: "FORUMS_EVENT(私域论坛事件)", mask: 1 << 28 },
  { name: "AUDIO_ACTION(频道直播)", mask: 1 << 29 },
  { name: "PUBLIC_GUILD_MESSAGES(频道艾特)", mask: 1 << 30 },
];
// 与 mashiro-core gateway.rs 的 DEFAULT_INTENTS 保持一致
const DEFAULT_INTENTS = (1 << 30) | (1 << 12) | (1 << 24) | (1 << 25) | (1 << 26);

interface AddVals {
  appid: number;
  secret: string;
  mode: number;
}

interface DetailVals {
  appid: number;
  secret: string;
  mode: number;
  sandbox: boolean;
  md_group: boolean;
  md_guild: boolean;
  md_guild_tpl: boolean;
  ws_url: string;
}

export default function AccountsPage() {
  const { message, modal } = App.useApp();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [bots, setBots] = useState<BotInfo[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  const [addOpen, setAddOpen] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [intentOpen, setIntentOpen] = useState(false);
  const [detailAcc, setDetailAcc] = useState<Account | null>(null);
  const [intentAcc, setIntentAcc] = useState<Account | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; acc: Account } | null>(null);
  const [addForm] = Form.useForm<AddVals>();
  const [detailForm] = Form.useForm<DetailVals>();
  const [intentForm] = Form.useForm<{ ws_intents: number }>();
  const detailMode = Form.useWatch("mode", detailForm);
  const formIntents = Form.useWatch("ws_intents", intentForm) ?? 0;
  const intentSelCount = INTENTS.filter((i) => (formIntents & i.mask) !== 0).length;

  // ---------- 扫码登录 ----------
  const [qrOpen, setQrOpen] = useState(false);
  const [qrUrl, setQrUrl] = useState("");
  const [qrTip, setQrTip] = useState("使用手机QQ扫码绑定");
  const taskRef = useRef<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopPoll = () => {
    if (pollRef.current) clearInterval(pollRef.current);
    pollRef.current = null;
    taskRef.current = null;
  };

  const startQr = async () => {
    try {
      const r = await api.qrBindStart();
      taskRef.current = r.task_id;
      setQrUrl(r.qr_url);
      setQrTip("使用手机QQ扫码绑定");
      setQrOpen(true);
      pollRef.current = setInterval(async () => {
        const taskId = taskRef.current;
        if (!taskId) return;
        try {
          const p = await api.qrBindPoll(taskId);
          if (p.status === "done" && p.appid && p.secret) {
            stopPoll();
            setQrOpen(false);
            setAddOpen(false);
            try {
              // 对齐原版：扫码成功直接建档（默认配置）并自动连接
              await api.saveAccount({
                appid: p.appid,
                secret: p.secret,
                name: "",
                avatar: "",
                enabled: true,
                mode: 0,
                sandbox: false,
                md_group: true,
                md_guild: false,
                md_guild_tpl: false,
                ws_intents: DEFAULT_INTENTS,
                ws_url: "",
              });
              message.success(`扫码成功，账号 ${p.appid} 已添加`);
              refresh();
              api.startBot(p.appid).catch(() => {});
            } catch (e) {
              message.error(String(e));
            }
          } else if (p.status === "expired") {
            stopPoll();
            setQrTip("二维码已过期，请重新生成");
          }
        } catch {
          /* 网络抖动静默重试 */
        }
      }, 2000);
    } catch (e) {
      message.error(String(e));
    }
  };

  useEffect(() => {
    if (qrOpen && qrUrl && canvasRef.current) {
      QRCode.toCanvas(canvasRef.current, qrUrl, { width: 240, margin: 1 }).catch(() => {});
    }
  }, [qrOpen, qrUrl]);

  useEffect(() => stopPoll, []);

  // ---------- 列表 ----------
  const refresh = useCallback(() => {
    api.listAccounts().then(setAccounts).catch((e) => message.error(String(e)));
    api.botStatus().then(setBots).catch(() => {});
  }, [message]);
  useEffect(refresh, [refresh]);

  // 在线状态 3s 轮询
  useEffect(() => {
    const t = setInterval(() => api.botStatus().then(setBots).catch(() => {}), 3000);
    return () => clearInterval(t);
  }, []);

  const selAcc = accounts.find((a) => a.appid === selected) ?? null;
  const selBot = bots.find((b) => b.appid === selected);
  const online = selBot?.online ?? false;

  // ---------- 添加 ----------
  const openAdd = () => {
    addForm.setFieldsValue({ appid: 0, secret: "", mode: 0 });
    setAddOpen(true);
  };

  const saveAdd = async () => {
    const v = await addForm.validateFields();
    setBusy(true);
    try {
      await api.saveAccount({
        appid: v.appid,
        secret: v.secret,
        name: "",
        avatar: "",
        enabled: true,
        mode: v.mode,
        sandbox: false,
        md_group: true,
        md_guild: false,
        md_guild_tpl: false,
        ws_intents: DEFAULT_INTENTS,
        ws_url: "",
      });
      message.success("已添加，去「详情」可配沙盒/Markdown/订阅事件");
      setAddOpen(false);
      refresh();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ---------- 详情（账号配置） ----------
  const openDetail = (a: Account) => {
    setDetailAcc(a);
    detailForm.setFieldsValue({
      appid: a.appid,
      secret: a.secret || SECRET_MASK,
      mode: a.mode,
      sandbox: a.sandbox,
      md_group: a.md_group,
      md_guild: a.md_guild,
      md_guild_tpl: a.md_guild_tpl,
      ws_url: a.ws_url,
    });
    setDetailOpen(true);
  };

  const saveDetail = async () => {
    if (!detailAcc) return;
    const v = await detailForm.validateFields();
    setBusy(true);
    try {
      await api.saveAccount({
        ...detailAcc,
        secret: v.secret,
        mode: v.mode,
        sandbox: v.sandbox,
        md_group: v.md_group,
        md_guild: v.md_guild,
        md_guild_tpl: v.md_guild_tpl,
        ws_url: v.ws_url ?? "",
      });
      message.success("已保存");
      setDetailOpen(false);
      refresh();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ---------- 订阅配置 ----------
  const openIntents = (a: Account) => {
    setIntentAcc(a);
    intentForm.setFieldsValue({ ws_intents: a.ws_intents || DEFAULT_INTENTS });
    setIntentOpen(true);
  };

  const saveIntents = async () => {
    if (!intentAcc) return;
    const v = await intentForm.validateFields();
    setBusy(true);
    try {
      await api.saveAccount({
        ...intentAcc,
        secret: SECRET_MASK, // 占位：后端保留原值
        ws_intents: v.ws_intents,
      });
      message.success("已保存");
      setIntentOpen(false);
      refresh();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (appid: number) => {
    try {
      await api.removeAccount(appid);
      if (selected === appid) {
        setSelected(null);
        setDrawerOpen(false);
      }
      refresh();
    } catch (e) {
      message.error(String(e));
    }
  };

  const toggleBot = async (appid: number) => {
    try {
      const isOn = bots.find((b) => b.appid === appid)?.online ?? false;
      if (isOn) {
        await api.stopBot(appid);
      } else {
        await api.startBot(appid);
      }
      refresh();
    } catch (e) {
      message.error(String(e));
    }
  };

  const intentCount = selAcc ? INTENTS.filter((i) => (selAcc.ws_intents & i.mask) !== 0).length : 0;

  const botOf = (appid: number) => bots.find((b) => b.appid === appid);
  const intentOf = (a: Account) => INTENTS.filter((i) => (a.ws_intents & i.mask) !== 0).length;

  return (
    <div>
      <h2 className="page-title">账号管理</h2>
      <div className="card">
        <div className="acct-head-bar">
          <span className="mat-sub">
            共 {accounts.length} 个托管账号 · 点击卡片查看详情 · 右键呼出菜单
          </span>
          <div className="spacer" />
          <Button onClick={startQr}>扫码登录</Button>
          <Button type="primary" onClick={openAdd}>
            添加账号
          </Button>
        </div>
        {accounts.length === 0 ? (
          <div className="plg-hint" style={{ padding: "36px 0" }}>
            还没有托管账号，点「添加账号」或「扫码登录」开始
          </div>
        ) : (
          <div className="acct-grid">
            {accounts.map((a) => {
              const b = botOf(a.appid);
              const on = b?.online ?? false;
              return (
                <div
                  className={`acct-card${a.appid === selected ? " sel" : ""}`}
                  key={a.appid}
                  onClick={() => {
                    setSelected(a.appid);
                    setDrawerOpen(true);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setCtxMenu({ x: e.clientX, y: e.clientY, acc: a });
                  }}
                >
                  <div className="acct-card-top">
                    <BotAvatar appid={a.appid} name={a.name} avatar={b?.avatar || a.avatar} size={42} />
                    <div className="acct-card-meta">
                      <div className="acct-card-name">
                        {b?.name || a.name || `Bot ${a.appid}`}
                      </div>
                      <div className="acct-card-appid">{a.appid}</div>
                    </div>
                    <span className={`acct-state${on ? " on" : ""}`}>
                      <i className={`dot ${on ? "online" : "offline"}`} />
                      {on ? "在线" : "离线"}
                    </span>
                  </div>
                  <div className="acct-card-tags">
                    <Tag color={a.mode === 1 ? "purple" : "blue"}>
                      {a.mode === 1 ? "Webhook" : a.ws_url ? "WS 中转" : "WebSocket"}
                    </Tag>
                    {a.sandbox && <Tag color="orange">沙盒</Tag>}
                    {!a.enabled && <Tag>停用</Tag>}
                    {a.mode === 0 && <Tag>订阅 {intentOf(a)} 项</Tag>}
                  </div>
                  <div className="acct-card-ops" onClick={(e) => e.stopPropagation()}>
                    <Button
                      size="small"
                      type={on ? "default" : "primary"}
                      danger={on}
                      onClick={() => toggleBot(a.appid)}
                    >
                      {on ? "停止" : "启动"}
                    </Button>
                    <Button size="small" onClick={() => openDetail(a)}>
                      配置
                    </Button>
                    <Button size="small" onClick={() => openIntents(a)}>
                      订阅
                    </Button>
                    <Popconfirm title="删除该账号？" onConfirm={() => remove(a.appid)}>
                      <Button size="small" danger type="text">
                        删除
                      </Button>
                    </Popconfirm>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* 账号详情抽屉：点行拉出，默认收起 */}
      <Drawer
        title={selAcc ? `账号详情 · ${selAcc.appid}` : "账号详情"}
        placement="right"
        width={300}
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
      >
        {selAcc && (
          <>
            <div className="acct-head">
              <BotAvatar appid={selAcc.appid} name={selAcc.name} avatar={selAcc.avatar} size={40} />
              <div className="acct-head-meta">
                <div className="nm">{selBot?.name || selAcc.name || `Bot ${selAcc.appid}`}</div>
                <div className="sub">AppID {selAcc.appid}</div>
              </div>
            </div>
            <div className="acct-kv">
              <span>状态</span>
              <b>
                <i className={`dot ${online ? "online" : "offline"}`} />
                {online ? "在线" : "离线"}
              </b>
            </div>
            <div className="acct-kv">
              <span>连接方式</span>
              <b>
                {selAcc.mode === 1 ? "Webhook" : selAcc.ws_url ? "WS 中转" : "WebSocket"}
              </b>
            </div>
            {selAcc.mode === 0 && selAcc.ws_url && (
              <div className="acct-kv">
                <span>中转地址</span>
                <b className="mono" style={{ fontSize: 11, wordBreak: "break-all" }}>
                  {selAcc.ws_url}
                </b>
              </div>
            )}
            <div className="acct-kv">
              <span>沙盒环境</span>
              <b>{selAcc.sandbox ? "开启" : "关闭"}</b>
            </div>
            <div className="acct-kv">
              <span>Markdown</span>
              <b>
                {[
                  selAcc.md_group && "群",
                  selAcc.md_guild && "频道",
                  selAcc.md_guild_tpl && "模板",
                ]
                  .filter(Boolean)
                  .join(" / ") || "关"}
              </b>
            </div>
            {selAcc.mode === 0 && (
              <div className="acct-kv">
                <span>订阅事件</span>
                <b>{intentCount} 项</b>
              </div>
            )}
            <Space direction="vertical" style={{ width: "100%", marginTop: 14 }}>
              <Button block type="primary" onClick={() => toggleBot(selAcc.appid)}>
                {online ? "停止" : "启动"}
              </Button>
              <Button
                block
                onClick={() => {
                  setDrawerOpen(false);
                  openDetail(selAcc);
                }}
              >
                账号配置
              </Button>
              <Button
                block
                onClick={() => {
                  setDrawerOpen(false);
                  openIntents(selAcc);
                }}
              >
                订阅事件
              </Button>
            </Space>
          </>
        )}
      </Drawer>

      {/* 添加账号：只填最小三项，其余去详情配 */}
      <Modal
        title="添加账号"
        open={addOpen}
        onOk={saveAdd}
        confirmLoading={busy}
        onCancel={() => setAddOpen(false)}
        okText="保存"
        cancelText="取消"
        width={480}
      >
        <Form form={addForm} layout="vertical" style={{ marginTop: 12 }}>
          <Form.Item label="AppID" required style={{ marginBottom: 10 }}>
            <Space.Compact style={{ width: "100%" }}>
              <Form.Item name="appid" noStyle rules={[{ required: true, message: "必填" }]}>
                <InputNumber style={{ flex: 1 }} placeholder="机器人 AppID" />
              </Form.Item>
              <Button onClick={startQr}>扫码登录</Button>
            </Space.Compact>
          </Form.Item>
          <Form.Item
            name="secret"
            label="Secret"
            rules={[{ required: true, message: "必填" }]}
            style={{ marginBottom: 10 }}
          >
            <Input.Password placeholder="机器人 AppSecret" />
          </Form.Item>
          <Form.Item name="mode" label="连接方式" style={{ marginBottom: 0 }} initialValue={0}>
            <Radio.Group>
              <Radio value={0}>WebSocket</Radio>
              <Radio value={1}>Webhook</Radio>
            </Radio.Group>
          </Form.Item>
        </Form>
      </Modal>

      {/* 账号详情：连接/沙盒/Markdown/订阅事件（对齐原版 AddAccountDialog 其余字段） */}
      <Modal
        title="账号详细设置"
        open={detailOpen}
        onOk={saveDetail}
        confirmLoading={busy}
        onCancel={() => setDetailOpen(false)}
        okText="保存"
        cancelText="取消"
        width={620}
      >
        <Form form={detailForm} layout="vertical" style={{ marginTop: 12 }}>
          <Space size={12} style={{ display: "flex" }}>
            <Form.Item name="appid" label="AppID" style={{ marginBottom: 10, width: 180 }}>
              <InputNumber style={{ width: "100%" }} disabled />
            </Form.Item>
            <Form.Item name="secret" label="Secret" style={{ marginBottom: 10, flex: 1 }}>
              <Input.Password placeholder="留空保持不变" />
            </Form.Item>
          </Space>

          <div className="set-group">
            <div className="set-group-title">连接</div>
            <Form.Item name="mode" style={{ marginBottom: 6 }}>
              <Radio.Group>
                <Radio value={0}>WebSocket</Radio>
                <Radio value={1}>Webhook</Radio>
              </Radio.Group>
            </Form.Item>
            {detailMode === 0 && (
              <Form.Item name="ws_url" style={{ marginBottom: 6 }}>
                <Input placeholder="中转网关地址 wss://…（可选，webhook→websocket 中转；留空走官方 /gateway）" />
              </Form.Item>
            )}
            <Form.Item name="sandbox" valuePropName="checked" style={{ marginBottom: 0 }}>
              <Checkbox>沙盒环境（腾讯沙盒）</Checkbox>
            </Form.Item>
          </div>

          <div className="set-group">
            <div className="set-group-title">Markdown 能力</div>
            <Space size={16}>
              <Form.Item name="md_group" valuePropName="checked" noStyle>
                <Checkbox>群 Markdown</Checkbox>
              </Form.Item>
              <Form.Item name="md_guild" valuePropName="checked" noStyle>
                <Checkbox>频道原生 Markdown</Checkbox>
              </Form.Item>
              <Form.Item name="md_guild_tpl" valuePropName="checked" noStyle>
                <Checkbox>频道模板</Checkbox>
              </Form.Item>
            </Space>
          </div>
        </Form>
      </Modal>

      {/* 订阅配置：独立的 intents 选择弹窗 */}
      <Modal
        title={intentAcc ? `订阅事件 · ${intentAcc.appid}` : "订阅事件"}
        open={intentOpen}
        onOk={saveIntents}
        confirmLoading={busy}
        onCancel={() => setIntentOpen(false)}
        okText="保存"
        cancelText="取消"
        width={560}
      >
        <Form form={intentForm} style={{ marginTop: 12 }}>
          <div className="dlg-intents-head" style={{ marginBottom: 10 }}>
            <span className="set-hint" style={{ marginTop: 0 }}>
              WebSocket / 中转模式订阅的事件类型（Webhook 模式以平台后台配置为准）
            </span>
            <span className="dlg-intents-meta">
              已选 {intentSelCount} 项
              <a onClick={() => intentForm.setFieldValue("ws_intents", DEFAULT_INTENTS)}>
                恢复默认
              </a>
            </span>
          </div>
          <div className="intents-grid intents-grid-full">
            {INTENTS.map((it) => (
              <Checkbox
                key={it.mask}
                checked={(formIntents & it.mask) !== 0}
                onChange={(e) => {
                  const cur = intentForm.getFieldValue("ws_intents") ?? 0;
                  intentForm.setFieldValue(
                    "ws_intents",
                    e.target.checked ? cur | it.mask : cur & ~it.mask,
                  );
                }}
              >
                {it.name}
              </Checkbox>
            ))}
          </div>
          <Form.Item name="ws_intents" hidden>
            <InputNumber />
          </Form.Item>
        </Form>
      </Modal>

      {/* 行右键菜单 */}
      {ctxMenu && (
        <div
          className="ctx-mask"
          onClick={() => setCtxMenu(null)}
          onContextMenu={(e) => {
            e.preventDefault();
            setCtxMenu(null);
          }}
        >
          <div
            className="ctx-menu"
            style={{
              left: Math.min(ctxMenu.x, window.innerWidth - 170),
              top: Math.min(ctxMenu.y, window.innerHeight - 180),
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div
              className="ctx-item"
              onClick={() => {
                setCtxMenu(null);
                toggleBot(ctxMenu.acc.appid);
              }}
            >
              {bots.find((b) => b.appid === ctxMenu.acc.appid)?.online ? "停止" : "启动"}
            </div>
            <div
              className="ctx-item"
              onClick={() => {
                setCtxMenu(null);
                openDetail(ctxMenu.acc);
              }}
            >
              账号配置
            </div>
            <div
              className="ctx-item"
              onClick={() => {
                setCtxMenu(null);
                openIntents(ctxMenu.acc);
              }}
            >
              订阅事件
            </div>
            <div className="ctx-sep" />
            <div
              className="ctx-item danger"
              onClick={() => {
                const appid = ctxMenu.acc.appid;
                setCtxMenu(null);
                modal.confirm({
                  title: "删除该账号？",
                  content: `AppID ${appid} 及其配置将一并移除`,
                  okText: "删除",
                  okButtonProps: { danger: true },
                  cancelText: "取消",
                  onOk: () => remove(appid),
                });
              }}
            >
              删除账号
            </div>
          </div>
        </div>
      )}

      {/* 扫码绑定弹窗 */}
      <Modal
        title="扫描二维码登录"
        open={qrOpen}
        footer={null}
        onCancel={() => {
          stopPoll();
          setQrOpen(false);
        }}
        width={280}
      >
        <div style={{ textAlign: "center", padding: "8px 0 4px" }}>
          <canvas ref={canvasRef} />
          <div style={{ marginTop: 10, color: "var(--text-sub)", fontSize: 12.5 }}>{qrTip}</div>
        </div>
      </Modal>
    </div>
  );
}
