import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Tooltip } from "antd";
import dayjs from "dayjs";
import {
  api,
  type Account,
  type BotInfo,
  type LogRow,
  type Stats,
} from "../api";
import { BotAvatar } from "../components/TitleBar";
import appIcon from "../../../src-tauri/icons/128x128.png";

const fmt = (n: number) =>
  n >= 10000 ? `${(n / 10000).toFixed(1)}万` : n.toLocaleString();

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
const KIND_COLOR: Record<string, string> = {
  Error: "#f3312f",
  Send: "#d46a2a",
  System: "#999",
  Plugin: "#7b61ff",
  Group: "#2f9e6e",
  C2C: "#4a8fd4",
};

function greet(): string {
  const h = new Date().getHours();
  if (h < 5) return "夜深了";
  if (h < 9) return "早上好";
  if (h < 12) return "上午好";
  if (h < 14) return "中午好";
  if (h < 18) return "下午好";
  return "晚上好";
}

function StatCard(props: {
  icon: string;
  bg: string;
  title: string;
  value: string;
  sub?: string;
}) {
  return (
    <div className="stat-card">
      <div className="stat-icon" style={{ background: props.bg }}>
        {props.icon}
      </div>
      <div className="stat-text">
        <div className="stat-title">
          {props.title}
          {props.sub && <span className="stat-sub">{props.sub}</span>}
        </div>
        <div className="stat-value">{props.value}</div>
      </div>
    </div>
  );
}

export default function HomePage() {
  const nav = useNavigate();
  const [bots, setBots] = useState<BotInfo[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [events, setEvents] = useState<LogRow[]>([]);

  const refresh = useCallback(() => {
    api.botStatus().then(setBots).catch(() => {});
    api.getStats().then(setStats).catch(() => {});
    api.listAccounts().then(setAccounts).catch(() => {});
    api.listLogs(Object.keys(KIND_NAMES), 0, 10).then(setEvents).catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);

  const online = bots.filter((b) => b.online).length;
  const maxHour = Math.max(1, ...(stats?.hourly ?? []).map(([r, s]) => r + s));
  const botOf = (appid: number) => bots.find((b) => b.appid === appid);

  return (
    <div className="home-page">
      {/* 顶部问候横幅 */}
      <div className="home-hero">
        <div className="hero-text">
          <div className="hero-hi">
            {greet()}，{dayjs().format("M月D日")} {["周日","周一","周二","周三","周四","周五","周六"][dayjs().day()]}
          </div>
          <div className="hero-sub">
            {accounts.length === 0
              ? "还没有托管账号，去「账号」页添加一个吧"
              : online > 0
                ? `${online} 个机器人在线运行中，今日已处理 ${fmt(stats?.msgs_today ?? 0)} 条消息`
                : "暂无机器人在线，去「账号」页启动一个吧"}
          </div>
        </div>
        <img className="hero-icon" src={appIcon} alt="" draggable={false} />
      </div>

      {/* 统计小卡 */}
      <div className="stat-row">
        <StatCard
          icon="💬"
          bg="#E8F4FD"
          title="今日消息"
          value={fmt(stats?.msgs_today ?? 0)}
          sub={`收 ${fmt(stats?.recv_today ?? 0)} · 发 ${fmt(stats?.sent_today ?? 0)}`}
        />
        <StatCard
          icon="🤖"
          bg="#F3E5F5"
          title="在线账号"
          value={`${stats?.online ?? online} / ${stats?.accounts ?? accounts.length}`}
          sub="在线 / 托管"
        />
        <StatCard
          icon="🧩"
          bg="#FDF1E3"
          title="已加载插件"
          value={String(stats?.plugins_loaded ?? 0)}
          sub="运行中宿主"
        />
        <StatCard
          icon="⚡"
          bg="#FDEBE8"
          title="插件触发"
          value={fmt(
            (stats?.top_plugins ?? []).reduce((a, [, n]) => a + n, 0),
          )}
          sub="今日指令命中"
        />
      </div>

      {/* 中排：消息趋势 + 插件 TOP */}
      <div className="home-mid">
        <div className="card home-chart">
          <div className="home-panel-head">
            <span className="home-panel-title">消息趋势</span>
            <span className="home-pill">收 {fmt(stats?.recv_today ?? 0)}</span>
            <span className="home-pill">发 {fmt(stats?.sent_today ?? 0)}</span>
            <span className="home-panel-sub">每小时收发量</span>
          </div>
          <div className="chart-area">
            {(stats?.hourly ?? []).map(([r, s], h) => (
              <Tooltip
                key={h}
                title={`${h} 时：收 ${r} · 发 ${s}`}
                mouseEnterDelay={0}
              >
                <div className="chart-col">
                  <div className="chart-bars">
                    <i
                      className="bar recv"
                      style={{ height: `${(r / maxHour) * 100}%` }}
                    />
                    <i
                      className="bar send"
                      style={{ height: `${(s / maxHour) * 100}%` }}
                    />
                  </div>
                  {h % 4 === 0 && <span className="chart-h">{h}</span>}
                </div>
              </Tooltip>
            ))}
          </div>
        </div>

        <div className="card home-top">
          <div className="home-panel-head">
            <span className="home-panel-title">插件消息 TOP 6</span>
          </div>
          <div className="home-rank">
            {(stats?.top_plugins ?? []).length === 0 ? (
              <div className="plg-hint" style={{ padding: "20px 0" }}>
                今天还没有插件被触发
              </div>
            ) : (
              (stats?.top_plugins ?? []).map(([name, n], i) => {
                const max = stats!.top_plugins[0]?.[1] || 1;
                return (
                  <div className="rank-row" key={name}>
                    <span className={`rank-no ${i < 3 ? "top" : ""}`}>{i + 1}</span>
                    <span className="rank-name">{name || "（未命名）"}</span>
                    <span className="rank-n">{fmt(n)}</span>
                    <span className="rank-bar">
                      <i style={{ width: `${(n / max) * 100}%` }} />
                    </span>
                  </div>
                );
              })
            )}
          </div>
        </div>
      </div>

      {/* 底排：机器人状态 + 最近事件 */}
      <div className="home-bottom">
        <div className="card home-bots">
          <div className="home-panel-head">
            <span className="home-panel-title">机器人</span>
            <span className="home-panel-sub">
              {online}/{accounts.length} 在线
            </span>
          </div>
          <div className="bot-list">
            {accounts.length === 0 ? (
              <div className="plg-hint" style={{ padding: "20px 0" }}>
                暂无托管账号
              </div>
            ) : (
              accounts.map((a) => {
                const b = botOf(a.appid);
                const on = b?.online ?? false;
                return (
                  <div className="bot-row" key={a.appid}>
                    <BotAvatar
                      appid={a.appid}
                      name={a.name}
                      avatar={b?.avatar}
                      size={30}
                    />
                    <div className="bot-meta">
                      <span className="bot-name">{a.name || `Bot ${a.appid}`}</span>
                      <span className="bot-appid">{a.appid}</span>
                    </div>
                    <span className="bot-mode">{a.mode === 0 ? "WS" : "Hook"}</span>
                    <span className={`bot-state ${on ? "on" : ""}`}>
                      <i className={`dot ${on ? "online" : "offline"}`} />
                      {on ? "在线" : "离线"}
                    </span>
                    <button
                      className={`bot-toggle${on ? " stop" : ""}`}
                      onClick={() =>
                        (on ? api.stopBot(a.appid) : api.startBot(a.appid)).then(
                          refresh,
                        )
                      }
                    >
                      {on ? "停止" : "启动"}
                    </button>
                  </div>
                );
              })
            )}
          </div>
        </div>

        <div className="card home-events">
          <div className="home-panel-head">
            <span className="home-panel-title">最近事件</span>
            <span
              className="home-panel-sub link"
              onClick={() => nav("/logs")}
            >
              全部日志 →
            </span>
          </div>
          <div className="evt-list">
            {events.length === 0 ? (
              <div className="plg-hint" style={{ padding: "20px 0" }}>
                还没有事件
              </div>
            ) : (
              events.map((e) => (
                <div
                  className="evt-row"
                  key={e.id}
                  onClick={() => nav("/logs")}
                  title={e.content}
                >
                  <span
                    className="evt-kind"
                    style={{ color: KIND_COLOR[e.kind] ?? "var(--text-sub)" }}
                  >
                    {KIND_NAMES[e.kind] ?? e.kind}
                  </span>
                  <span className="evt-text">{e.content || "—"}</span>
                  <span className="evt-ts">{dayjs(e.ts).format("HH:mm")}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
