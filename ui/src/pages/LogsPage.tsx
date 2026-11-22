import { useCallback, useEffect, useRef, useState } from "react";
import {
  App,
  Button,
  Checkbox,
  Dropdown,
  Empty,
  InputNumber,
  Modal,
  Select,
  Tooltip,
} from "antd";
import type { MenuProps } from "antd";
import dayjs from "dayjs";
import { api, type Account, type LogRow } from "../api";

const KIND_NAMES: Record<string, string> = {
  Group: "群聊",
  Guild: "频道",
  C2C: "私聊",
  GuildDm: "频道私聊",
  Send: "发送",
  Error: "错误",
  System: "事件",
  Plugin: "插件",
};
const ALL_KINDS = Object.keys(KIND_NAMES);
const KIND_COLOR: Record<string, string> = {
  Error: "#f3312f",
  Send: "#d46a2a",
  System: "#999",
  Plugin: "#7b61ff",
};

function short(s: string, n = 10): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}

/** 日志的完整 JSON：raw（插件收到的标准化事件）优先，否则退化为行字段。 */
function logJson(r: LogRow): string {
  if (r.raw) {
    try {
      return JSON.stringify(JSON.parse(r.raw), null, 2);
    } catch {
      return r.raw;
    }
  }
  const { raw: _, ...row } = r;
  return JSON.stringify(row, null, 2);
}

export default function LogsPage() {
  const { message } = App.useApp();
  const [rows, setRows] = useState<LogRow[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [appid, setAppid] = useState(0);
  const [limit, setLimit] = useState(300);
  const [hiddenKinds, setHiddenKinds] = useState<Set<string>>(new Set());
  const [realtime, setRealtime] = useState(true);
  const [detail, setDetail] = useState<LogRow | null>(null);
  const [jsonView, setJsonView] = useState<LogRow | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true); // 用户是否停在底部

  const kinds = ALL_KINDS.filter((k) => !hiddenKinds.has(k));

  const refresh = useCallback(() => {
    api
      .listLogs(ALL_KINDS.filter((k) => !hiddenKinds.has(k)), appid, limit)
      // 接口返回 id DESC；展示为旧→新（终端式），新行出现在底部
      .then((rs) => setRows([...rs].reverse()))
      .catch((e) => message.error(String(e)));
  }, [appid, limit, hiddenKinds, message]);

  useEffect(() => {
    api.listAccounts().then(setAccounts).catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
    if (!realtime) return;
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh, realtime]);

  // 实时滚动：新数据进来且用户本来就在底部才吸底（翻上去阅读不打断）
  useEffect(() => {
    if (realtime && stickRef.current && bodyRef.current) {
      bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
    }
  }, [rows, realtime]);

  const botName = (id: number) =>
    accounts.find((a) => a.appid === id)?.name || (id ? String(id) : "-");

  const copy = (text: string, what: string) => {
    navigator.clipboard
      .writeText(text)
      .then(() => message.success(`${what}已复制`))
      .catch(() => message.error("复制失败"));
  };

  const delRow = async (id: number) => {
    try {
      await api.deleteLog(id);
      setRows((rs) => rs.filter((r) => r.id !== id));
    } catch (e) {
      message.error(String(e));
    }
  };

  const clearAll = async () => {
    try {
      await api.clearLogs();
      setRows([]);
      message.success("日志已清空");
    } catch (e) {
      message.error(String(e));
    }
  };

  const rowMenu = (r: LogRow): MenuProps["items"] => [
    {
      key: "copy",
      label: "复制信息",
      onClick: () => copy(r.content, "内容"),
    },
    { key: "view", label: "查看信息", onClick: () => setDetail(r) },
    {
      key: "json",
      label: "查看 JSON",
      onClick: () => setJsonView(r),
    },
    {
      key: "copy-json",
      label: "复制 JSON",
      onClick: () => copy(logJson(r), "JSON"),
    },
    { type: "divider" },
    {
      key: "copy-user",
      label: "复制发送人",
      disabled: !r.user,
      onClick: () => copy(r.user, "发送人"),
    },
    {
      key: "copy-bot",
      label: "复制主号",
      disabled: !r.appid,
      onClick: () => copy(String(r.appid), "机器人"),
    },
    {
      key: "copy-src",
      label: "复制来源",
      disabled: !r.group_id,
      onClick: () => copy(r.group_id, "会话"),
    },
    { type: "divider" },
    { key: "clear", label: "清空日志", onClick: clearAll },
    {
      key: "del",
      label: "删除此条",
      danger: true,
      onClick: () => delRow(r.id),
    },
    { type: "divider" },
    {
      key: "filter",
      label: "日志临时过滤选项",
      children: ALL_KINDS.map((k) => ({
        key: `f-${k}`,
        label: (
          <Checkbox
            checked={!hiddenKinds.has(k)}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              setHiddenKinds((s) => {
                const next = new Set(s);
                if (e.target.checked) next.delete(k);
                else next.add(k);
                return next;
              });
            }}
          >
            {KIND_NAMES[k]}
          </Checkbox>
        ),
      })),
    },
  ];

  return (
    <div>
      <h2 className="page-title">日志</h2>
      <div className="card log-card">
        <div className="log-toolbar">
          <span className="log-tb-label">机器人</span>
          <Select
            size="small"
            style={{ width: 150 }}
            value={appid}
            onChange={setAppid}
            options={[
              { value: 0, label: "全部账号" },
              ...accounts.map((a) => ({
                value: a.appid,
                label: a.name || String(a.appid),
              })),
            ]}
          />
          <span className="log-tb-label">加载数量</span>
          <InputNumber
            size="small"
            min={50}
            max={5000}
            step={50}
            value={limit}
            onChange={(v) => setLimit(v ?? 300)}
            onBlur={refresh}
            onPressEnter={refresh}
            style={{ width: 80 }}
          />
          {hiddenKinds.size > 0 && (
            <span className="log-filter-hint">
              已过滤 {hiddenKinds.size} 类（右键行可改）
            </span>
          )}
        </div>

        <div className="log-table">
          <div className="log-head">
            <span className="lc-time">时间</span>
            <span className="lc-bot">机器人</span>
            <span className="lc-kind">类型</span>
            <span className="lc-group">会话</span>
            <span className="lc-user">发送人</span>
            <span className="lc-msg">内容</span>
          </div>
          <div
            className="log-body"
            ref={bodyRef}
            onScroll={(e) => {
              const el = e.currentTarget;
              stickRef.current =
                el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            }}
          >
            {rows.length === 0 && (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description="暂无日志"
                style={{ marginTop: 60 }}
              />
            )}
            {rows.map((r) => (
              <Dropdown
                key={r.id}
                trigger={["contextMenu"]}
                menu={{ items: rowMenu(r) }}
              >
                <div
                  className="log-row"
                  style={{ color: KIND_COLOR[r.kind] || undefined }}
                >
                  <span className="lc-time">
                    {dayjs(r.ts).format("HH:mm:ss")}
                  </span>
                  <span className="lc-bot">
                    <Tooltip title={r.appid}>{short(botName(r.appid), 8)}</Tooltip>
                  </span>
                  <span className="lc-kind">{KIND_NAMES[r.kind] ?? r.kind}</span>
                  <span className="lc-group">
                    <Tooltip title={r.group_id}>{short(r.group_id, 8)}</Tooltip>
                  </span>
                  <span className="lc-user">
                    <Tooltip title={r.user}>{short(r.user, 10)}</Tooltip>
                  </span>
                  <span className="lc-msg" title={r.content}>
                    {r.content}
                  </span>
                </div>
              </Dropdown>
            ))}
          </div>
        </div>

        <div className="log-foot">
          <span className="log-count">{rows.length} 条 · {kinds.length}/{ALL_KINDS.length} 类</span>
          <div className="log-foot-r">
            <Checkbox
              checked={realtime}
              onChange={(e) => setRealtime(e.target.checked)}
            >
              日志实时滚动
            </Checkbox>
            <Button size="small" onClick={refresh}>
              刷新
            </Button>
            <Button size="small" danger onClick={clearAll}>
              清空
            </Button>
          </div>
        </div>
      </div>

      <Modal
        title="日志详情"
        open={detail !== null}
        footer={null}
        onCancel={() => setDetail(null)}
      >
        {detail && (
          <div className="log-detail">
            <div className="log-detail-row">
              <span>时间</span>
              <b>{dayjs(detail.ts).format("YYYY-MM-DD HH:mm:ss.SSS")}</b>
            </div>
            <div className="log-detail-row">
              <span>机器人</span>
              <b>
                {botName(detail.appid)}（{detail.appid || "-"}）
              </b>
            </div>
            <div className="log-detail-row">
              <span>类型</span>
              <b>{KIND_NAMES[detail.kind] ?? detail.kind}</b>
            </div>
            <div className="log-detail-row">
              <span>会话</span>
              <b>{detail.group_id || "-"}</b>
            </div>
            <div className="log-detail-row">
              <span>发送人</span>
              <b>{detail.user || "-"}</b>
            </div>
            <div className="log-detail-content">{detail.content}</div>
          </div>
        )}
      </Modal>

      <Modal
        title={jsonView?.raw ? "事件 JSON" : "日志 JSON"}
        open={jsonView !== null}
        footer={
          <Button
            size="small"
            onClick={() => jsonView && copy(logJson(jsonView), "JSON")}
          >
            复制
          </Button>
        }
        onCancel={() => setJsonView(null)}
        width={560}
      >
        {jsonView && <pre className="log-json mono">{logJson(jsonView)}</pre>}
      </Modal>
    </div>
  );
}
