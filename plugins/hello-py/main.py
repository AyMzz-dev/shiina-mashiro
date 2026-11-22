import mashiro

bot = mashiro.Bot()


@bot.command("签到")
def signin(ctx):
    n = ctx.kv.incr("score")
    ctx.reply(f"签到成功，积分 {n}")
