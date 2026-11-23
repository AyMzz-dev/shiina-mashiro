import { invoke } from "@tauri-apps/api/core";

export interface Account {
  appid: number;
  secret: string;
  name: string;
  avatar: string;
  enabled: boolean;
  /** 0 WebSocket / 1 Webhook */
  mode: number;
  sandbox: boolean;
  md_group: boolean;
  md_guild: boolean;
  md_guild_tpl: boolean;
  /** WS intents 浣嶆帺鐮侊紝0 = 榛樿璁㈤槄 */
  ws_intents: number;
  /** 自定义 WS 网关地址（webhook→websocket 中转），空 = 官方 /gateway */
  ws_url: string;
}

export interface QrBindStart {
  task_id: string;
  qr_url: string;
}

export interface QrBindPoll {
  status: "waiting" | "expired" | "done";
  appid?: number;
  secret?: string;
}

export interface BotInfo {
  appid: number;
  name: string;
  avatar: string;
  online: boolean;
}

export interface LogRow {
  id: number;
  ts: number;
  appid: number;
  kind: string;
  group_id: string;
  user: string;
  content: string;
  /** 标准化事件完整 JSON；非事件来源为空串 */
  raw: string;
}

export interface PluginRow {
  uuid: string;
  dir: string;
  runtime: string;
  name: string;
  enabled: boolean;
  config: string; // JSON锛歿 commands: string[] }
}

export interface Stats {
  accounts: number;
  online: number;
  msgs_today: number;
  plugins_loaded: number;
  recv_today: number;
  sent_today: number;
  hourly: [number, number][];
  top_plugins: [string, number][];
}

export const api = {
  greet: () => invoke<string>("greet"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  saveAccount: (account: Account) =>
    invoke<void>("save_account", { account }),
  qrBindStart: () => invoke<QrBindStart>("qr_bind_start"),
  qrBindPoll: (taskId: string) =>
    invoke<QrBindPoll>("qr_bind_poll", { taskId }),
  removeAccount: (appid: number) => invoke<void>("remove_account", { appid }),
  botStatus: () => invoke<BotInfo[]>("bot_status"),
  startBot: (appid: number) => invoke<void>("start_bot", { appid }),
  stopBot: (appid: number) => invoke<void>("stop_bot", { appid }),
  getStats: () => invoke<Stats>("get_stats"),

  listPlugins: () => invoke<PluginRow[]>("list_plugins"),
  pluginHandlers: (pluginId: string) =>
    invoke<string[]>("plugin_handlers", { pluginId }),
  installPlugin: (dir: string) =>
    invoke<{ id: string; name: string; handlers: string[] }>("install_plugin", { dir }),
  scaffoldPlugin: (id: string, name: string, runtime: string) =>
    invoke<{ id: string; name: string; handlers: string[] }>("scaffold_plugin", {
      id, name, runtime,
    }),
  setPluginEnabled: (uuid: string, enabled: boolean) =>
    invoke<void>("set_plugin_enabled", { uuid, enabled }),
  unloadPlugin: (uuid: string) => invoke<void>("unload_plugin", { uuid }),
  reloadPlugin: (uuid: string) => invoke<void>("reload_plugin", { uuid }),
  setPluginAccounts: (uuid: string, accounts: number[]) =>
    invoke<void>("set_plugin_accounts", { uuid, accounts }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  removePlugin: (uuid: string) => invoke<void>("remove_plugin", { uuid }),

  getSetting: (key: string) => invoke<string>("get_setting", { key }),
  checkProgram: (path: string) => invoke<string>("check_program", { path }),
  dataDir: () => invoke<string>("data_dir"),
  /** PEM 内容落盘为 tls/webhook-{kind}.pem，返回路径 */
  writePemFile: (kind: "cert" | "key", content: string) =>
    invoke<string>("write_pem_file", { kind, content }),
  setSetting: (key: string, value: string) =>
    invoke<void>("set_setting", { key, value }),

  listLogs: (kinds: string[], appid = 0, limit = 200) =>
    invoke<LogRow[]>("list_logs", { kinds, appid, limit }),
  deleteLog: (id: number) => invoke<void>("delete_log", { id }),
  clearLogs: () => invoke<void>("clear_logs"),
  purgeLogs: () => invoke<number>("purge_logs"),

  listTimers: () => invoke<TimerRow[]>("list_timers"),
  setTimerEnabled: (id: number, enabled: boolean) =>
    invoke<void>("set_timer_enabled", { id, enabled }),
  removeTimer: (id: number) => invoke<void>("remove_timer", { id }),

  listConversations: () => invoke<Conversation[]>("list_conversations"),
  chatHistory: (appid: number, groupId: string, beforeId = 0, limit = 100) =>
    invoke<LogRow[]>("chat_history", { appid, groupId, beforeId, limit }),
  sendChat: (p: {
    appid: number;
    openid: string;
    kind: number;
    content: string;
    markdown: boolean;
    keyboard?: unknown;
    msgId?: string;
  }) => invoke<void>("send_chat", { ...p }),

  materialList: (appid: number, cursor = "") =>
    invoke<MaterialList>("material_list", { appid, cursor }),
  materialUpload: (appid: number, path: string) =>
    invoke<MaterialImage>("material_upload", { appid, path }),
  materialDelete: (appid: number, imageId: string) =>
    invoke<void>("material_delete", { appid, imageId }),

  pluginConfigSchema: (pluginId: string) =>
    invoke<ConfigField[]>("plugin_config_schema", { pluginId }),
  pluginConfigGet: (pluginId: string) =>
    invoke<Record<string, unknown>>("plugin_config_get", { pluginId }),
  pluginConfigSet: (pluginId: string, values: Record<string, unknown>) =>
    invoke<void>("plugin_config_set", { pluginId, values }),

  groupBotState: (appid: number, groupOpenid: string) =>
    invoke<GroupBotState>("group_bot_state", { appid, groupOpenid }),
  sendAuthCard: (appid: number, groupOpenid: string, msgId?: string) =>
    invoke<void>("send_auth_card", { appid, groupOpenid, msgId }),

  agentGenerate: (session: string | null, runtime: string, prompt: string) =>
    invoke<{ session: string; files: AgentFile[]; text?: string }>(
      "agent_generate",
      { session, runtime, prompt },
    ),
  agentInstall: (session: string) =>
    invoke<{ id: string; name: string; handlers: string[] }>("agent_install", {
      session,
    }),
  agentModels: (provider: string, base: string, key: string) =>
    invoke<string[]>("agent_models", { provider, base, key }),
  agentTest: (provider: string, base: string, key: string, model: string) =>
    invoke<string>("agent_test", { provider, base, key, model }),
  agentAnswer: (callId: string, answers: string[]) =>
    invoke<void>("agent_answer", { callId, answers }),
  agentSessions: () => invoke<AgentSessionInfo[]>("agent_sessions"),
  agentSessionState: (session: string) =>
    invoke<{
      runtime: string;
      files: AgentFile[];
      turns: { role: "user" | "agent"; text: string }[];
      active: boolean;
    }>("agent_session_state", { session }),
  agentSessionDelete: (session: string) =>
    invoke<void>("agent_session_delete", { session }),
  agentCancel: (session: string) => invoke<void>("agent_cancel", { session }),
  agentAttachPlugin: (session: string | null, pluginId: string) =>
    invoke<{ session: string; files: AgentFile[]; runtime: string }>(
      "agent_attach_plugin",
      { session, pluginId },
    ),
  agentPendingAsk: (session: string) =>
    invoke<{ call_id: string; questions: AskQuestion[] } | null>(
      "agent_pending_ask",
      { session },
    ),
};

export interface AgentSessionInfo {
  id: string;
  title: string;
  runtime: string;
  updated_at: number;
}

export interface AskQuestion {
  q: string;
  options: string[];
}

export type ConfigFieldTy =
  | "string" | "textarea" | "password" | "number" | "slider" | "bool"
  | "select" | "radio" | "multi_select" | "tags"
  | "path" | "file" | "color" | "date" | "time";

export interface ConfigField {
  key: string;
  label: string;
  ty: ConfigFieldTy;
  default?: unknown;
  options?: string[];
  desc?: string;
  password?: boolean;
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
  required?: boolean;
}

export interface AgentFile {
  path: string;
  content: string;
}

export interface TimerRow {
  id: number;
  plugin_id: string;
  remark: string;
  next_run: number;
  repeat_secs: number;
  enabled: boolean;
}

export interface Conversation {
  appid: number;
  group_id: string;
  kind: string;
  last_content: string;
  last_ts: number;
  count: number;
}

export interface MaterialImage {
  image_id: string;
  url: string;
  preview_url?: string;
  file_name?: string;
  [k: string]: unknown;
}

export interface MaterialList {
  images?: MaterialImage[];
  next_cursor?: string;
  total?: number;
}

export interface GroupBotState {
  member_openid?: string;
  joined_at?: string;
  allow_proactive_msg?: boolean;
  recv_msg_setting?: string;
  member_role?: string;
}
