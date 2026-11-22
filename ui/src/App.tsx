import { Route, Routes } from "react-router-dom";
import SideBar from "./components/SideBar";
import TopBar from "./components/TitleBar";
import HomePage from "./pages/HomePage";
import AccountsPage from "./pages/AccountsPage";
import PluginsPage from "./pages/PluginsPage";
import LogsPage from "./pages/LogsPage";
import SettingsPage from "./pages/SettingsPage";
import AboutPage from "./pages/AboutPage";
import AgentPage from "./pages/AgentPage";
import ChatPage from "./pages/ChatPage";
import TasksPage from "./pages/TasksPage";
import MaterialPage from "./pages/MaterialPage";

export default function App() {
  return (
    <div className="shell">
      <SideBar />
      <div className="main">
        <TopBar />
        <div className="content">
          <Routes>
            <Route path="/" element={<HomePage />} />
            <Route path="/accounts" element={<AccountsPage />} />
            <Route path="/plugins" element={<PluginsPage />} />
            <Route path="/chat" element={<ChatPage />} />
            <Route path="/tasks" element={<TasksPage />} />
            <Route path="/material" element={<MaterialPage />} />
            <Route path="/agent" element={<AgentPage />} />
            <Route path="/logs" element={<LogsPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/about" element={<AboutPage />} />
          </Routes>
        </div>
      </div>
    </div>
  );
}
