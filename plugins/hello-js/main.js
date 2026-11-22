const bot = require("mashiro");

bot.command("签到", async (ctx) => {
  const n = await ctx.kv.incr("score");
  await ctx.reply(`签到成功，积分 ${n}`);
});
