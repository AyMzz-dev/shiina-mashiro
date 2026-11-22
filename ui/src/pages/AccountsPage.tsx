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
  Table,
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
const DEFAULT_INTENTS = (1 << 30) | (1 << 12) | (1 << 25);

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
  ws_intents: number;
}

export default function AccountsPage() {
  const { message } = App.useApp();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [bots, setBots] = useState<BotInfo[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  const [addOpen, setAddOpen] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [addForm] = Form.useForm<AddVals>();
  const [detailForm] = Form.useForm<DetailVals>();
  const detailMode = Form.useWatch("mode", detailForm);
  const detailIntents = Form.useWatch("ws_intents", detailForm) ?? 0;

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

  // ---------- 详情 ----------
  const openDetail = (a: Account) => {
    detailForm.setFieldsValue({
      appid: a.appid,
      secret: a.secret || SECRET_MASK,
      mode: a.mode,
      sandbox: a.sandbox,
      md_group: a.md_group,
      md_guild: a.md_guild,
      md_guild_tpl: a.md_guild_tpl,
      ws_intents: a.ws_intents || DEFAULT_INTENTS,
    });
    setDetailOpen(true);
  };

  const saveDetail = async () => {
    const v = await detailForm.validateFields();
    setBusy(true);
    try {
      await api.saveAccount({
        appid: v.appid,
        secret: v.secret,
        name: "",
        avatar: "",
        enabled: true,
        mode: v.mode,
        sandbox: v.sandbox,
        md_group: v.md_group,
        md_guild: v.md_guild,
        md_guild_tpl: v.md_guild_tpl,
        ws_intents: v.ws_intents,
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
      if (online) {
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

  return (
    <div>
      <h2 className="page-title">账号管理</h2>
      <div className="card">
        <Space style={{ marginBottom: 12 }}>
          <Button type="primary" onClick={openAdd}>
            添加账号
          </Button>
        </Space>
        <Table<Account>
          rowKey="appid"
          size="middle"
          dataSource={accounts}
          pagination={false}
          rowClassName={(r) => (r.appid === selected ? "acct-row-sel" : "")}
          onRow={(r) => ({
            onClick: () => {
              setSelected(r.appid);
              setDrawerOpen(true);
            },
            style: { cursor: "pointer" },
          })}
          columns={[
            { title: "AppID", dataIndex: "appid" },
            {
              title: "名称",
              dataIndex: "name",
              render: (v: string, r) => v || `Bot ${r.appid}`,
            },
            {
              title: "连接",
              dataIndex: "mode",
              render: (v: number, r) => (
                <>
                  <Tag color={v === 1 ? "purple" : "blue"}>
                    {v === 1 ? "Webhook" : "WebSocket"}
                  </Tag>
                  {r.sandbox && <Tag color="orange">沙盒</Tag>}
                </>
              ),
            },
            {
              title: "启用",
              dataIndex: "enabled",
              render: (v: boolean) =>
                v ? <Tag color="green">启用</Tag> : <Tag>停用</Tag>,
            },
            {
              title: "操作",
              render: (_, r) => (
                <Popconfirm title="删除该账号？" onConfirm={() => remove(r.appid)}>
                  <Button size="small" danger>
                    删除
                  </Button>
                </Popconfirm>
              ),
            },
          ]}
        />
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
              <b>{selAcc.mode === 1 ? "Webhook" : "WebSocket"}</b>
            </div>
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
                详细配置
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
            <Form.Item name="secret" label="Secret" style={{ marginBottom: 10, flex: 1, width: 300 }}>
              <Input.Password placeholder="留空保持不变" />
            </Form.Item>
          </Space>
          <Form.Item name="mode" label="连接方式" style={{ marginBottom: 6 }}>
            <Radio.Group>
              <Radio value={0}>WebSocket</Radio>
              <Radio value={1}>Webhook</Radio>
            </Radio.Group>
          </Form.Item>
          <Form.Item name="sandbox" valuePropName="checked" style={{ marginBottom: 6 }}>
            <Checkbox>连接沙盒（腾讯沙盒环境）</Checkbox>
          </Form.Item>
          <Space size={16} style={{ marginBottom: 10, display: "flex" }}>
            <Form.Item name="md_group" valuePropName="checked" noStyle>
              <Checkbox>群Markdown</Checkbox>
            </Form.Item>
            <Form.Item name="md_guild" valuePropName="checked" noStyle>
              <Checkbox>频道原生Markdown</Checkbox>
            </Form.Item>
            <Form.Item name="md_guild_tpl" valuePropName="checked" noStyle>
              <Checkbox>频道模板</Checkbox>
            </Form.Item>
          </Space>

          <div className="intents-box">
            <div className="intents-title">订阅事件 (WebSocket)</div>
            <div className="intents-grid" style={{ opacity: detailMode === 1 ? 0.45 : 1 }}>
              {INTENTS.map((it) => (
                <Checkbox
                  key={it.mask}
                  disabled={detailMode === 1}
                  checked={(detailIntents & it.mask) !== 0}
                  onChange={(e) => {
                    const cur = detailForm.getFieldValue("ws_intents") ?? 0;
                    detailForm.setFieldValue(
                      "ws_intents",
                      e.target.checked ? cur | it.mask : cur & ~it.mask,
                    );
                  }}
                >
                  {it.name}
                </Checkbox>
              ))}
            </div>
          </div>
          <Form.Item name="ws_intents" hidden>
            <InputNumber />
          </Form.Item>
        </Form>
      </Modal>

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
