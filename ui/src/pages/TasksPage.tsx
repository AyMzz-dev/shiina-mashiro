import { useCallback, useEffect, useState } from "react";
import { App, Button, Empty, Popconfirm, Switch, Table, Tag, Tooltip } from "antd";
import dayjs from "dayjs";
import { api, type PluginRow, type TimerRow } from "../api";

function fmtRepeat(secs: number): string {
  if (secs <= 0) return "一次性";
  if (secs % 86400 === 0) return `每 ${secs / 86400} 天`;
  if (secs % 3600 === 0) return `每 ${secs / 3600} 小时`;
  if (secs % 60 === 0) return `每 ${secs / 60} 分钟`;
  return `每 ${secs} 秒`;
}

export default function TasksPage() {
  const { message } = App.useApp();
  const [rows, setRows] = useState<TimerRow[]>([]);
  const [plugins, setPlugins] = useState<PluginRow[]>([]);

  const refresh = useCallback(() => {
    api.listTimers().then(setRows).catch((e) => message.error(String(e)));
    api.listPlugins().then(setPlugins).catch(() => {});
  }, [message]);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, [refresh]);

  const nameOf = (pid: string) =>
    plugins.find((p) => p.uuid === pid)?.name || pid;

  return (
    <div>
      <h2 className="page-title">任务</h2>
      <div className="card task-card">
        <div className="task-head">
          <b>定时任务</b>
          <span className="task-sub">
            插件通过 add_timer 注册，到期投递 event:timer · {rows.length} 条
          </span>
          <div className="spacer" />
          <Button size="small" onClick={refresh}>
            刷新
          </Button>
        </div>
        <Table<TimerRow>
          size="small"
          rowKey="id"
          dataSource={rows}
          pagination={false}
          locale={{ emptyText: <Empty description="暂无定时任务" /> }}
          columns={[
            {
              title: "插件",
              dataIndex: "plugin_id",
              width: 180,
              render: (v: string) => <b>{nameOf(v)}</b>,
            },
            {
              title: "备注",
              dataIndex: "remark",
              ellipsis: true,
              render: (v: string) => v || <span style={{ color: "#bbb" }}>—</span>,
            },
            {
              title: "下次执行",
              dataIndex: "next_run",
              width: 170,
              render: (v: number) => {
                const past = v * 1000 < Date.now();
                return (
                  <span className="mono" style={past ? { color: "#d46a2a" } : undefined}>
                    {dayjs(v * 1000).format("MM-DD HH:mm:ss")}
                    {past && <Tooltip title="已过期，等待轮询触发"> ⚠</Tooltip>}
                  </span>
                );
              },
            },
            {
              title: "周期",
              dataIndex: "repeat_secs",
              width: 110,
              render: (v: number) =>
                v > 0 ? <Tag color="blue">{fmtRepeat(v)}</Tag> : <Tag>一次性</Tag>,
            },
            {
              title: "启用",
              dataIndex: "enabled",
              width: 70,
              render: (v: boolean, r) => (
                <Switch
                  size="small"
                  checked={v}
                  onChange={async (on) => {
                    try {
                      await api.setTimerEnabled(r.id, on);
                      refresh();
                    } catch (e) {
                      message.error(String(e));
                    }
                  }}
                />
              ),
            },
            {
              title: "",
              width: 70,
              render: (_, r) => (
                <Popconfirm
                  title="删除该定时任务？"
                  onConfirm={async () => {
                    try {
                      await api.removeTimer(r.id);
                      refresh();
                    } catch (e) {
                      message.error(String(e));
                    }
                  }}
                >
                  <Button size="small" danger type="text">
                    删除
                  </Button>
                </Popconfirm>
              ),
            },
          ]}
        />
      </div>
    </div>
  );
}
