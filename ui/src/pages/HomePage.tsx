import { useCallback, useEffect, useState } from "react";
import { Tooltip } from "antd";
import { api, type BotInfo, type Stats } from "../api";

const fmt = (n: number) =>
  n >= 10000 ? `${(n / 10000).toFixed(1)}万` : n.toLocaleString();

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
  const [bots, setBots] = useState<BotInfo[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);

  const refresh = useCallback(() => {
    api.botStatus().then(setBots).catch(() => {});
    api.getStats().then(setStats).catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);

  const online = bots.filter((b) => b.online).length;
  const maxHour = Math.max(1, ...(stats?.hourly ?? []).map(([r, s]) => r + s));

  return (
    <div className="home-page">
      <h2 className="page-title">首页</h2>

      {/* 统计小卡（对齐原版：图标块 + 标题 + 值 + 副值） */}
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
          value={`${stats?.online ?? online} / ${stats?.accounts ?? 0}`}
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

      {/* 消息趋势 + 插件 TOP（原版中排左右结构，撑满剩余高度） */}
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
    </div>
  );
}
