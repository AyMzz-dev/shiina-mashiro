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

    // 主动发送 / 被动回复（带 msg_id 无频次限制）
    send: (text) => call("send_message", sendBase({ text })),
    reply: (text) => call("send_message", sendBase({ text, msg_id: event.msg_id })),
    sendMarkdown: (content) => call("send_markdown", sendBase({ content })),
    replyMarkdown: (content) =>
      call("send_markdown", sendBase({ content, msg_id: event.msg_id })),
    sendMedia: (fileType, url) =>
      call("send_media", sendBase({ file_type: fileType, url, msg_id: event.msg_id })),
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

  /** 在线 bot 列表 */
  bots: () => call("bot_list"),
  /** openid ↔ 整数短 ID */
  shortId: (appid, user) => call("get_short_id", { appid, user }),
  openidOf: (appid, shortId) => call("get_openid", { appid, short_id: shortId }),

  /** 频道(guild)域接口，QQ 群无官方群管 API */
  guild: {
    info: (appid, guildId) => call("guild_info", { appid, guild_id: guildId }),
    members: (appid, guildId, limit = 100, after = "0") =>
      call("get_member_list", { appid, guild_id: guildId, limit, after }),
    member: (appid, guildId, userId) =>
      call("get_member_info", { appid, guild_id: guildId, user_id: userId }),
    kick: (appid, guildId, userId, deleteMsgDays = 0) =>
      call("kick_member", { appid, guild_id: guildId, user_id: userId, delete_msg_days: deleteMsgDays }),
  },

  /** Markdown 图片素材：上传后拿 qqbot-media:// 链接，插进 markdown 里发图（media 权限） */
  material: {
    list: (appid, cursor = "") => call("material_list", { appid, cursor }),
    upload: (appid, path) => call("material_upload", { appid, path }),
    del: (appid, imageId) => call("material_delete", { appid, image_id: imageId }),
  },

  /** 通用 HTTP 逃生舱 → {status, body} */
  http: (opts) => call("http_request", opts),
};
