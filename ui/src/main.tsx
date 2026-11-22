import React from "react";
import ReactDOM from "react-dom/client";
import { App as AntdApp, ConfigProvider } from "antd";
import { HashRouter } from "react-router-dom";
import App from "./App";
import "./index.css";

// 桌面应用不需要 WebView 原生右键菜单；放过输入框/可编辑区（复制粘贴菜单）。
// antd Dropdown 的 contextMenu 走 React 合成事件，preventDefault 不影响它。
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (t?.closest("input, textarea, [contenteditable='true'], pre, code")) return;
  e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("app")!).render(
  <React.StrictMode>
    <ConfigProvider
      theme={{
        token: {
          colorPrimary: "#ff7f32",
          colorBgLayout: "#f7efe5",
          colorText: "#263241",
          colorTextSecondary: "#687589",
          borderRadius: 8,
        },
      }}
    >
      <AntdApp>
        <HashRouter>
          <App />
        </HashRouter>
      </AntdApp>
    </ConfigProvider>
  </React.StrictMode>,
);
