import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "./Icons";
import appIcon from "../../../src-tauri/icons/32x32.png";

const win = getCurrentWindow();

const titles: Record<string, string> = {
  "/": "首页",
  "/accounts": "账号",
  "/plugins": "插件",
  "/chat": "聊天",
  "/tasks": "任务",
  "/material": "素材",
  "/agent": "生成",
  "/logs": "日志",
  "/settings": "设置",
  "/about": "关于",
};

/** 由 appid 派生头像配色（对应原版 generateBotAvatar 的思路） */
function avatarColor(appid: number): string {
  const hue = Math.abs(appid * 2654435761) % 360;
  return `linear-gradient(135deg, hsl(${hue},70%,60%), hsl(${(hue + 40) % 360},70%,50%))`;
}

export function BotAvatar({
  appid,
  name,
  avatar,
  size = 30,
}: {
  appid: number;
  name: string;
  avatar?: string;
  size?: number;
}) {
  if (avatar) {
    return (
      <img
        className="avatar"
        src={avatar}
        style={{ width: size, height: size, objectFit: "cover" }}
        onError={(e) => {
          (e.target as HTMLImageElement).style.display = "none";
        }}
      />
    );
  }
  return (
    <div
      className="avatar"
      style={{ width: size, height: size, background: avatarColor(appid), fontSize: size * 0.44 }}
    >
      {(name || "真").charAt(0)}
    </div>
  );
}

/** 一言兜底（一言 API 不可达时随机取本地动漫句） */
const localQuotes = [
  { text: "只要我记得你，你就永远不会消失。", from: "你的名字。" },
  { text: "重要的东西全部都在半径三米之内。", from: "CLANNAD" },
  { text: "就算如此，我也要继续前进。", from: "进击的巨人" },
  { text: "愿你有一天能和重要的人重逢。", from: "可塑性记忆" },
  { text: "没有未来的未来，不是我想要的未来。", from: "境界的彼方" },
  { text: "我喜欢朋也，比世界上任何一个人都喜欢。", from: "CLANNAD" },
  { text: "世界虽然残酷，但依然美丽。", from: "进击的巨人" },
  { text: "因为不想忘记，所以要一直记得。", from: "四月是你的谎言" },
];

function useHitokoto() {
  const [q, setQ] = useState(
    () => localQuotes[Math.floor(Math.random() * localQuotes.length)],
  );
  useEffect(() => {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), 4000);
    fetch("https://v1.hitokoto.cn/?c=a&c=b&c=c&max_length=30", { signal: ctl.signal })
      .then((r) => r.json())
      .then((d) => {
        if (d?.hitokoto) setQ({ text: d.hitokoto, from: d.from_who || d.from || "" });
      })
      .catch(() => {})
      .finally(() => clearTimeout(t));
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  }, []);
  return q;
}

/** 右侧内容区上方的细条：拖拽区 + 当前页名 + 一言 + 窗口按钮 */
export default function TopBar() {
  const { pathname } = useLocation();
  const quote = useHitokoto();
  return (
    <div className="topbar" data-tauri-drag-region>
      <img className="tb-icon" src={appIcon} alt="" draggable={false} />
      <span className="crumb" data-tauri-drag-region>
        椎名真白 · {titles[pathname] ?? ""}
      </span>
      <span className="tb-quote" title={`${quote.text} —— ${quote.from}`}>
        {quote.text}
        {quote.from ? ` · ${quote.from}` : ""}
      </span>
      <div className="spacer" data-tauri-drag-region />
      <div className="win-btns">
        <button className="win-btn" title="重启" onClick={() => window.location.reload()}>
          <Icon name="restart" size={13} />
        </button>
        <button className="win-btn" title="最小化" onClick={() => win.minimize()}>
          <svg width="12" height="12" viewBox="0 0 12 12">
            <path d="M1 6h10" stroke="currentColor" strokeWidth="1.4" />
          </svg>
        </button>
        <button className="win-btn close" title="关闭" onClick={() => win.close()}>
          <svg width="11" height="11" viewBox="0 0 11 11">
            <path d="M1 1l9 9M10 1l-9 9" stroke="currentColor" strokeWidth="1.4" />
          </svg>
        </button>
      </div>
    </div>
  );
}
