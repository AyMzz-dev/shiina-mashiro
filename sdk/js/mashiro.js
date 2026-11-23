"use strict";
/**
 * Mashiro JS 插件 SDK。
 * 在 host worker 环境里运行：传输层由 worker.js 注入 globalThis.__mashiro_transport。
 *
 * 插件写法：
 *   const bot = require("mashiro");
 *   bot.command("签到", async (ctx) => { ctx.reply(`积分 ${await ctx.kv.incr("s")}`) });
 *   bot.on("timer", (ctx) => { ... });
 *   bot.timer(3600, 3600, "每小时");        // event:timer
 *   const v = await bot.http.get("https://...");
 */

const t = () => globalThis.__mashiro_transport;
const call = (m, p = {}) => t().callApi(m, p);

function wrapCtx(event) {
  const sendBase = (extra) => ({
    appid: event.appid,
    kind: event.kind,
    openid: event.group_id,
    ...extra,
  });
  return {
    event,
    appid: event.appid,
    kind: event.kind,
    groupId: event.group_id,
    guildId: event.guild_id,
    user: event.user,
    userInt: event.user_int,
    nickname: event.nickname,
    text: event.text,
    msgId: event.msg_id,
    atMe: event.at_me,
    memberRole: event.member_role,

    // 主动发送 / 被动回复（带 msg_id 无频次限制）。
    // opts 可带 event_id / msg_seq / is_wakeup / keyboard / recall_after_secs。
    send: (text, opts) => call("send_message", sendBase({ text, ...opts })),
    reply: (text, opts) =>
      call("send_message", sendBase({ text, msg_id: event.msg_id, ...opts })),
    sendMarkdown: (content, opts) => call("send_markdown", sendBase({ content, ...opts })),
    replyMarkdown: (content, opts) =>
      call("send_markdown", sendBase({ content, msg_id: event.msg_id, ...opts })),
    sendMedia: (fileType, url, opts) =>
      call("send_media", sendBase({ file_type: fileType, url, msg_id: event.msg_id, ...opts })),
    // 本地文件分片上传发送（群/单聊）：fileType 1 图/2 视频/3 语音/4 文件
    sendFile: (fileType, path, opts) =>
      call("send_media_file", sendBase({ file_type: fileType, path, msg_id: event.msg_id, ...opts })),
    // 图文卡片（msg_type 8）
    sendCard: (card, opts) =>
      call("send_card", sendBase({ ...card, msg_id: event.msg_id, ...opts })),
    // 单聊流式分块：fields {input_mode,input_state,index,content_type,content_raw,
    //   event_id|msg_id, stream_msg_id?, msg_seq?}；首块不带 stream_msg_id，用返回 id 续传
    sendStream: (fields) => call("send_stream", sendBase(fields)),
    // INTERACTION_CREATE 事件应答：code 0 成功/1 失败/2 频繁/3 重复/4 无权限/5 仅管理员
    respond: (code = 0) =>
      call("interaction_respond", {
        appid: event.appid,
        interaction_id: (event.raw && event.raw.id) || event.msg_id,
        code,
      }),
    recall: (msgId) =>
      call("recall", sendBase({ msg_id: msgId || event.msg_id })),
    log: (msg) => call("log", { appid: event.appid, msg }),

    kv: {
      get: (k) => call("kv_get", { key: k }),
      set: (k, v) => call("kv_set", { key: k, value: String(v) }),
      del: (k) => call("kv_del", { key: k }),
      incr: (k) => call("kv_incr", { key: k }),
    },
  };
}

module.exports = {
  /** 注册指令：框架按用户配置的触发词匹配后投递 handler = "cmd:<name>" */
  command(name, fn) {
    t().register(`cmd:${name}`, (ev) => fn(wrapCtx(ev)));
  },
  /** 订阅事件：handler = "event:<type>"（如 "timer"） */
  on(type, fn) {
    t().register(`event:${type}`, (ev) => fn(wrapCtx(ev)));
  },

  /** 注册定时任务：delaySecs 后首次触发；repeatSecs>0 则周期触发（走 event:timer） */
  timer: (delaySecs, repeatSecs = 0, remark = "") =>
    call("add_timer", { delay_secs: delaySecs, repeat_secs: repeatSecs, remark }),
  cancelTimer: (timerId) => call("cancel_timer", { timer_id: timerId }),

  /** 写一条插件日志到日志页（调试随手可用；level "info"|"warn"|"error"） */
  log: (msg, level = "info") => call("log", { msg: String(msg), level }),

  /** 在线 bot 列表 */
  bots: () => call("bot_list"),
  /** openid ↔ 整数短 ID */
  shortId: (appid, user) => call("get_short_id", { appid, user }),
  openidOf: (appid, shortId) => call("get_openid", { appid, short_id: shortId }),

  /** 频道(guild)域接口 */
  guild: {
    info: (appid, guildId) => call("guild_info", { appid, guild_id: guildId }),
    members: (appid, guildId, limit = 100, after = "0") =>
      call("get_member_list", { appid, guild_id: guildId, limit, after }),
    member: (appid, guildId, userId) =>
      call("get_member_info", { appid, guild_id: guildId, user_id: userId }),
    kick: (appid, guildId, userId, deleteMsgDays = 0) =>
      call("kick_member", { appid, guild_id: guildId, user_id: userId, delete_msg_days: deleteMsgDays }),
  },

  /** QQ 群管理（v2/groups，机器人需为群管理员；group_admin 权限） */
  group: {
    info: (appid, groupOpenid) => call("group_info", { appid, group_openid: groupOpenid }),
    members: (appid, groupOpenid, cursor = "") =>
      call("group_members", { appid, group_openid: groupOpenid, cursor }),
    member: (appid, groupOpenid, memberOpenid) =>
      call("group_member_info", { appid, group_openid: groupOpenid, member_openid: memberOpenid }),
    blacklist: (appid, groupOpenid, cursor = "", limit = 20) =>
      call("group_blacklist", { appid, group_openid: groupOpenid, cursor, limit }),
    /** op "add"|"del"，memberOpenids ≤20 */
    blacklistOp: (appid, groupOpenid, op, memberOpenids) =>
      call("group_blacklist_op", { appid, group_openid: groupOpenid, op, member_openids: memberOpenids }),
    kick: (appid, groupOpenid, memberOpenids) =>
      call("group_remove_members", { appid, group_openid: groupOpenid, member_openids: memberOpenids }),
    muteSetting: (appid, groupOpenid) =>
      call("group_mute_setting", { appid, group_openid: groupOpenid }),
    /** members: [{op:"add"|"update"|"del", member_openid, mute_expire_at:"RFC3339"}] ≤20 */
    muteMembers: (appid, groupOpenid, members) =>
      call("group_mute_members", { appid, group_openid: groupOpenid, members }),
    joinRequests: (appid, groupOpenid, cursor = "", limit = 20) =>
      call("group_join_requests", { appid, group_openid: groupOpenid, cursor, limit }),
    /** op "approve"|"decline"；decline 可带 rejectReason/addToBlacklist */
    approveJoin: (appid, groupOpenid, memberOpenid, op, opts = {}) =>
      call("group_approve_join", {
        appid, group_openid: groupOpenid, member_openid: memberOpenid, op,
        join_request_id: opts.joinRequestId,
        reject_reason: opts.rejectReason,
        add_to_member_blacklist: !!opts.addToBlacklist,
      }),
  },

  /** 入群自动审批策略（group_admin 权限） */
  strategy: {
    list: (appid, cursor = "", limit = 20) =>
      call("strategy_list", { appid, cursor, limit }),
    /** body: {group_openids?|group_ids?, is_enable?, expire_at?, remark?} */
    create: (appid, body) => call("strategy_create", { appid, ...body }),
    /** body: {is_enable?, expire_at?, group_action?, remark?} */
    patch: (appid, strategyId, body) =>
      call("strategy_patch", { appid, strategy_id: strategyId, ...body }),
    del: (appid, strategyId) => call("strategy_delete", { appid, strategy_id: strategyId }),
    execute: (appid, strategyId) => call("strategy_execute", { appid, strategy_id: strategyId }),
    /** op "add"|"del"，users 为 QQ 号码字符串数组（≤10000） */
    whitelist: (appid, strategyId, op, users) =>
      call("strategy_whitelist", { appid, strategy_id: strategyId, op, whitelist_users: users }),
  },

  /** 私聊自定义菜单（仅 C2C 场景；group_admin 权限） */
  menu: {
    get: (appid) => call("menu_get", { appid }),
    /** menu {items:[{name,type,send_message?|link?|switch?|sub_menu_items?}]} 覆盖式更新 */
    put: (appid, menu) => call("menu_put", { appid, menu }),
  },

  /** 指令面板（group_admin 权限） */
  panel: {
    /** scope: "c2c"|"group"|"channel"|"dm" */
    list: (appid, scope, cursor = "", limit = 20) =>
      call("panel_list", { appid, scope, cursor, limit }),
    create: (appid, body) => call("panel_create", { appid, ...body }),
    get: (appid, panelId) => call("panel_get", { appid, panel_id: panelId }),
    update: (appid, panelId, body) =>
      call("panel_update", { appid, panel_id: panelId, ...body }),
    del: (appid, panelId) => call("panel_delete", { appid, panel_id: panelId }),
    /** op "add"|"del"；userOpenids / groupOpenids 按 scope 二选一传 */
    target: (appid, panelId, op, { userOpenids = [], groupOpenids = [] } = {}) =>
      call("panel_target", { appid, panel_id: panelId, op, user_openids: userOpenids, group_openids: groupOpenids }),
  },

  /** 单聊流式分块 / 互动应答 / 用户资料 / 分享链接 */
  sendStream: (appid, userOpenid, fields) =>
    call("send_stream", { appid, openid: userOpenid, ...fields }),
  respondInteraction: (appid, interactionId, code = 0) =>
    call("interaction_respond", { appid, interaction_id: interactionId, code }),
  userInfo: (appid, userOpenid) => call("user_info", { appid, user_openid: userOpenid }),
  shareLink: (appid, callbackData = "") =>
    call("generate_url_link", { appid, callback_data: callbackData }),

  /** Markdown 图片素材：上传后拿 qqbot-media:// 链接，插进 markdown 里发图（media 权限） */
  material: {
    list: (appid, cursor = "") => call("material_list", { appid, cursor }),
    upload: (appid, path) => call("material_upload", { appid, path }),
    del: (appid, imageId) => call("material_delete", { appid, image_id: imageId }),
  },

  /** 通用 HTTP 逃生舱 → {status, body} */
  http: (opts) => call("http_request", opts),
};
