import { useCallback, useEffect, useRef, useState } from "react";
import { NavLink } from "react-router-dom";
import { api, type BotInfo } from "../api";
import { getVersion } from "@tauri-apps/api/app";
import { Icon } from "./Icons";
import { BotAvatar } from "./TitleBar";
import { Updater } from "./Updater";
import appIcon from "../../../src-tauri/icons/128x128.png";

const items = [
  { to: "/", icon: "home", label: "首页" },
  { to: "/accounts", icon: "account", label: "账号" },
  { to: "/plugins", icon: "plugin", label: "插件" },
  { to: "/chat", icon: "chat", label: "聊天" },
  { to: "/tasks", icon: "clock", label: "任务" },
  { to: "/material", icon: "image", label: "素材" },
  { to: "/agent", icon: "spark", label: "生成" },
  { to: "/logs", icon: "logs", label: "日志" },
  { to: "/settings", icon: "settings", label: "设置" },
  { to: "/about", icon: "about", label: "关于" },
];

export default function SideBar() {
  const [bots, setBots] = useState<BotInfo[]>([]);
  const [idx, setIdx] = useState(0);
  const [ver, setVer] = useState("");
  const [pop, setPop] = useState(false);
  const footRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    api.botStatus().then(setBots).catch(() => {});
  }, []);
  useEffect(() => {
    getVersion().then(setVer).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);

  useEffect(() => {
    if (!pop) return;
    const onDown = (e: MouseEvent) => {
      if (footRef.current && !footRef.current.contains(e.target as Node)) setPop(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPop(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [pop]);

  const cur = bots.length > 0 ? bots[idx % bots.length] : null;

  return (
    <aside className="sidebar">
      <div className="side-brand" data-tauri-drag-region>
        <img className="logo" src={appIcon} alt="" draggable={false} />
        <div className="brand-text">
          <span className="name">椎名真白</span>
          <span className="sub">Shiina Mashiro</span>
        </div>
      </div>

      <nav className="side-nav">
        {items.map((it) => (
          <NavLink
            key={it.to}
            to={it.to}
            end={it.to === "/"}
            className={({ isActive }) => `side-item${isActive ? " on" : ""}`}
          >
            <Icon name={it.icon} size={16} />
            {it.label}
          </NavLink>
        ))}
      </nav>

      <div className="side-foot" ref={footRef}>
        {cur ? (
          <div className="bot-wrap">
            {pop && (
              <div className="bot-pop">
                {bots.map((b, i) => (
                  <div
                    key={b.appid}
                    className={`bot-pop-item${i === idx % bots.length ? " on" : ""}`}
                    onClick={() => {
                      setIdx(i);
                      setPop(false);
                    }}
                  >
                    <BotAvatar appid={b.appid} name={b.name} avatar={b.avatar} size={26} />
                    <div className="meta">
                      <span className="name">{b.name || `Bot ${b.appid}`}</span>
                      <span className="sub">{b.appid}</span>
                    </div>
                    <i className={`dot ${b.online ? "online" : "offline"}`} />
                    {i === idx % bots.length && (
                      <span className="tick">
                        <Icon name="check" size={13} />
                      </span>
                    )}
                  </div>
                ))}
              </div>
            )}
            <div
              className={`bot-chip${pop ? " open" : ""}`}
              title={`${cur.appid} · 点击切换账号`}
              onClick={() => setPop((o) => !o)}
            >
              <BotAvatar appid={cur.appid} name={cur.name} avatar={cur.avatar} size={30} />
              <div className="meta">
                <span className="name">{cur.name || `Bot ${cur.appid}`}</span>
                <span className="state">
                  <i className={`dot ${cur.online ? "online" : "offline"}`} />
                  {cur.online ? "在线" : "离线"}
                </span>
              </div>
              <span className={`arrow${pop ? " up" : ""}`}>
                <Icon name="chevron" size={13} />
              </span>
            </div>
          </div>
        ) : (
          <div className="bot-chip empty">未启动机器人</div>
        )}
        <div className="ver">
          <span>v{ver}</span>
          <Updater manual />
        </div>
      </div>
    </aside>
  );
}
