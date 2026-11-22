import { useCallback, useEffect, useMemo, useState } from "react";
import {
  App,
  Button,
  Checkbox,
  Dropdown,
  Empty,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
  Tag,
  Tooltip,
} from "antd";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, type Account, type PluginRow } from "../api";

interface PluginMeta {
  commands: string[];
  version: string;
  author: string;
  description: string;
  permissions: string[];
  accounts: number[];
}

function metaOf(p: PluginRow): PluginMeta {
  try {
    const c = JSON.parse(p.config);
    return {
      commands: Array.isArray(c.commands) ? c.commands : [],
      version: c.version || "",
      author: c.author || "",
      description: c.description || "",
      permissions: Array.isArray(c.permissions) ? c.permissions : [],
      accounts: Array.isArray(c.accounts) ? c.accounts : [],
    };
  } catch {
    return { commands: [], version: "", author: "", description: "", permissions: [], accounts: [] };
  }
}

const RUNTIME_COLOR: Record<string, string> = {
  node: "#97CEEB",
  python: "#2EE89F",
};

const PERM_NAME: Record<string, string> = {
  send: "发送消息",
  recall: "撤回消息",
  media: "发送媒体",
  kv: "键值存储",
  timer: "定时任务",
  http: "网络请求",
  group_admin: "群管理",
};

export default function PluginsPage() {
  const { message } = App.useApp();
  const [plugins, setPlugins] = useState<PluginRow[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [pendingAuth, setPendingAuth] = useState<number[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [scaffoldOpen, setScaffoldOpen] = useState(false);
  const [permOpen, setPermOpen] = useState(false);
  const [scaffoldForm] = Form.useForm<{ id: string; name: string; runtime: string }>();

  const refresh = useCallback(() => {
    api.listPlugins().then(setPlugins).catch((e) => message.error(String(e)));
    api.listAccounts().then(setAccounts).catch(() => {});
  }, [message]);
  useEffect(refresh, [refresh]);

  const sel = useMemo(
    () => plugins.find((p) => p.uuid === selected) ?? null,
    [plugins, selected],
  );
  const selMeta = sel ? metaOf(sel) : null;

  // 选中插件变化时同步授权待保存状态
  useEffect(() => {
    setPendingAuth(selMeta ? [...selMeta.accounts] : null);
  }, [selected, plugins]); // eslint-disable-line react-hooks/exhaustive-deps

  const authDirty =
    selMeta !== null &&
    pendingAuth !== null &&
    JSON.stringify([...pendingAuth].sort()) !== JSON.stringify([...selMeta.accounts].sort());

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      message.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  // 对齐原版：直接弹系统目录选择器，选中即安装（runtime 由 plugin.json 声明）
  const install = async () => {
    const picked = await openDialog({
      directory: true,
      title: "选择插件目录（含 plugin.json + 入口文件）",
    });
    if (typeof picked !== "string" || !picked) return;
    await run(async () => {
      const r = await api.installPlugin(picked);
      message.success(`已安装 ${r.name}（${r.handlers.length} 个指令）`);
      setSelected(r.id);
      refresh();
    });
  };

  const scaffold = async () => {
    const v = await scaffoldForm.validateFields();
    await run(async () => {
      const r = await api.scaffoldPlugin(v.id.trim(), v.name ?? "", v.runtime);
      message.success(`已创建并安装 ${r.name}`);
      setScaffoldOpen(false);
      setSelected(r.id);
      refresh();
    });
  };

  const toggle = async (p: PluginRow, enabled: boolean) => {
    try {
      await api.setPluginEnabled(p.uuid, enabled);
      refresh();
    } catch (e) {
      message.error(String(e));
    }
  };

  const saveAuth = async () => {
    if (!sel || pendingAuth === null) return;
    try {
      await api.setPluginAccounts(sel.uuid, pendingAuth);
      message.success("授权已保存");
      refresh();
    } catch (e) {
      message.error(String(e));
    }
  };

  const remove = async (p: PluginRow) => {
    try {
      await api.removePlugin(p.uuid);
      if (selected === p.uuid) setSelected(null);
      refresh();
    } catch (e) {
      message.error(String(e));
    }
  };

  const toggleAuth = (appid: number, on: boolean) => {
    setPendingAuth((cur) => {
      if (cur === null) return cur;
      return on ? [...cur, appid] : cur.filter((a) => a !== appid);
    });
  };

  return (
    <div>
      <h2 className="page-title">插件管理</h2>
      <div className="plg-layout">
        {/* 左列：插件列表 + 底部按钮（对齐图二） */}
        <div className="card plg-col-list">
          <div className="plg-list-head">
            <span className="c-name">应用</span>
            <span className="c-type">类型</span>
            <span className="c-ver">版本</span>
            <span className="c-author">作者</span>
          </div>
          <div className="plg-items">
            {plugins.length === 0 && (
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有插件" />
            )}
            {plugins.map((p) => {
              const m = metaOf(p);
              return (
                <div
                  key={p.uuid}
                  className={`plg-row${p.uuid === selected ? " plg-row-sel" : ""}`}
                  onClick={() => setSelected(p.uuid)}
                  onDoubleClick={() => toggle(p, !p.enabled)}
                >
                  <span className="c-name">
                    <i className={`plg-dot ${p.enabled ? "on" : "off"}`} />
                    <span className="plg-item-name">{p.name}</span>
                  </span>
                  <span className="c-type">
                    <span
                      className="plg-badge"
                      style={{ background: RUNTIME_COLOR[p.runtime] ?? "#ccc" }}
                    >
                      {p.runtime === "node" ? "JS" : "Python"}
                    </span>
                  </span>
                  <span className="c-ver">{m.version ? `v${m.version}` : "—"}</span>
                  <span className="c-author">
                    <Tooltip title={m.author}>{m.author || "未知"}</Tooltip>
                  </span>
                </div>
              );
            })}
          </div>
          <div className="plg-note">
            双击行启用/停用插件；右侧勾选授权账号，留空 = 所有账号可用
          </div>
          <div className="plg-btns">
            <Button
              type="primary"
              disabled={!sel}
              loading={busy}
              onClick={() => sel && toggle(sel, !sel.enabled)}
            >
              {sel?.enabled ? "停用" : "启用"}
            </Button>
            <Dropdown
              menu={{
                items: [
                  { key: "reload", label: "重载插件", disabled: !sel },
                  { key: "dir", label: "打开目录", disabled: !sel },
                  { type: "divider" },
                  { key: "new", label: "新建插件" },
                ],
                onClick: ({ key }) => {
                  if (key === "reload")
                    run(async () => {
                      await api.reloadPlugin(sel!.uuid);
                      message.success("已重载");
                      refresh();
                    });
                  else if (key === "dir")
                    api.openPath(sel!.dir).catch((e) => message.error(String(e)));
                  else if (key === "new") setScaffoldOpen(true);
                },
              }}
            >
              <Button>菜单</Button>
            </Dropdown>
            <Popconfirm
              title="卸载该插件？"
              onConfirm={() => sel && remove(sel)}
              disabled={!sel}
            >
              <Button danger disabled={!sel}>
                卸载
              </Button>
            </Popconfirm>
            <Button loading={busy} onClick={install}>导入插件</Button>
          </div>
        </div>

        {/* 右列：详情面板（对齐图二"请选择应用"） */}
        <div className="card plg-col-detail">
          {!sel || !selMeta ? (
            <div className="plg-empty">请选择应用</div>
          ) : (
            <>
              <div className="plg-detail-scroll">
                <div className="plg-path">
                  <span className="plg-k">路径：</span>
                  <Tooltip title={sel.dir}>
                    <span className="plg-path-v">{sel.dir}</span>
                  </Tooltip>
                </div>
                <div className="plg-desc">{selMeta.description || "（插件未提供描述）"}</div>
                <div className="plg-sec-title">注册的指令</div>
                <div className="plg-cmds">
                  {selMeta.commands.length === 0 ? (
                    <span className="plg-hint">未注册指令/事件</span>
                  ) : (
                    selMeta.commands.map((c) => (
                      <Tag key={c} color="orange" style={{ marginBottom: 4 }}>
                        {c.replace(/^cmd:/, "")}
                      </Tag>
                    ))
                  )}
                </div>
                <div className="plg-sec-title">账号授权</div>
                <div className="plg-auth-list">
                  {accounts.length === 0 ? (
                    <span className="plg-hint">还没有账号</span>
                  ) : (
                    accounts.map((a) => (
                      <Checkbox
                        key={a.appid}
                        checked={pendingAuth?.includes(a.appid) ?? false}
                        onChange={(e) => toggleAuth(a.appid, e.target.checked)}
                      >
                        {a.name || String(a.appid)}
                      </Checkbox>
                    ))
                  )}
                </div>
              </div>
              <div className="plg-btns">
                <Button disabled={!sel} onClick={() => setPendingAuth([])}>
                  清除勾选
                </Button>
                <Button
                  type="primary"
                  disabled={!authDirty}
                  loading={busy}
                  onClick={saveAuth}
                >
                  保存设置
                </Button>
                <Button disabled={!sel} onClick={() => setPermOpen(true)}>
                  权限说明
                </Button>
              </div>
            </>
          )}
        </div>
      </div>

      {/* 权限说明弹窗 */}
      <Modal
        title={`权限说明 — ${sel?.name ?? ""}`}
        open={permOpen}
        footer={null}
        onCancel={() => setPermOpen(false)}
      >
        {selMeta && (
          <div style={{ marginTop: 8 }}>
            {selMeta.permissions.length === 0 ? (
              <div className="plg-hint">插件未声明权限</div>
            ) : (
              selMeta.permissions.map((perm) => (
                <div key={perm} className="plg-perm-row">
                  <Tag color="orange">{perm}</Tag>
                  <span>{PERM_NAME[perm] ?? "自定义能力"}</span>
                </div>
              ))
            )}
            <div className="plg-hint" style={{ marginTop: 12 }}>
              权限声明来自 plugin.json 的 permissions 字段；框架按声明的能力向插件开放对应
              API，未声明的能力调用会被拒绝。
            </div>
          </div>
        )}
      </Modal>

      <Modal
        title="新建插件脚手架"
        open={scaffoldOpen}
        onOk={scaffold}
        onCancel={() => setScaffoldOpen(false)}
        okText="创建并安装"
        cancelText="取消"
        confirmLoading={busy}
        destroyOnClose
      >
        <Form
          form={scaffoldForm}
          layout="vertical"
          initialValues={{ runtime: "node" }}
          style={{ marginTop: 12 }}
        >
          <Form.Item
            name="id"
            label="插件 ID（全局唯一，如 daily-signin）"
            rules={[{ required: true, message: "必填" }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="name" label="显示名（留空用 ID）">
            <Input />
          </Form.Item>
          <Form.Item name="runtime" label="运行时" rules={[{ required: true }]}>
            <Select
              options={[
                { value: "node", label: "Node.js" },
                { value: "python", label: "Python" },
              ]}
            />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
