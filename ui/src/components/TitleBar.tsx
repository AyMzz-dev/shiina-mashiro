import { useLocation } from "react-router-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "./Icons";

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

/** 右侧内容区上方的细条：拖拽区 + 当前页名 + 窗口按钮 */
export default function TopBar() {
  const { pathname } = useLocation();
  return (
    <div className="topbar" data-tauri-drag-region>
      <span className="crumb" data-tauri-drag-region>
        椎名真白 · {titles[pathname] ?? ""}
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
