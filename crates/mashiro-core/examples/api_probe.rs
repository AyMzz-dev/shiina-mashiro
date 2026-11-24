//! QQ Bot API v2 实测探针：用真实账号把 api.rs 的接口打一遍。
//!
//! 用法：
//!   cargo run -p mashiro-core --example api_probe -- [db_path] [group_openid] [user_openid]
//!
//! - 不传 openid：只跑「全局类」接口（token / 资料 / 菜单 / 面板 / 素材 / 分享链接 / 策略列表）
//! - group_openid：追加群域全部接口（含发消息+撤回、12MB 分片上传）
//! - user_openid：追加 C2C 域（单聊发送、流式、资料）
//!
//! SKIP = 测试条件不满足（不是实现问题）；FAIL 里平台业务码（如 11253 无权限）也如实打印。

use std::sync::{Arc, OnceLock};

use mashiro_core::bot::{api, client::BotClient};
use mashiro_core::router::dispatch::Dispatcher;
use mashiro_core::storage::db::Database;
use serde_json::{json, Value};

static mut PASS: usize = 0;
static mut FAIL: usize = 0;
static mut SKIP: usize = 0;

fn report(name: &str, r: &anyhow::Result<Value>) {
    unsafe {
        match r {
            Ok(v) => {
                PASS += 1;
                let s = v.to_string();
                println!("PASS  {name:<28} {}", &s[..s.len().min(160)]);
            }
            Err(e) => {
                FAIL += 1;
                println!("FAIL  {name:<28} {e:#}");
            }
        }
    }
}

fn report_unit(name: &str, r: &anyhow::Result<()>) {
    unsafe {
        match r {
            Ok(()) => {
                PASS += 1;
                println!("PASS  {name}");
            }
            Err(e) => {
                FAIL += 1;
                println!("FAIL  {name:<28} {e:#}");
            }
        }
    }
}

fn skip(name: &str, why: &str) {
    unsafe { SKIP += 1 };
    println!("SKIP  {name:<28} {why}");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let db_path = args
        .next()
        .unwrap_or_else(|| "target/debug/data/mashiro.db".into());
    let group = args.next().unwrap_or_default();
    let user = args.next().unwrap_or_default();
    // 被动回复用：真实事件的 msg_id（60 分钟有效）；member_openid 测群成员详情
    let msg_id = args.next().unwrap_or_default();
    let c2c_msg_id = args.next().unwrap_or_default();
    let member = args.next().unwrap_or_default();

    let db = Arc::new(Database::open(&db_path)?);
    let acc = db
        .list_accounts()?
        .into_iter()
        .find(|a| a.enabled)
        .expect("数据库里没有已启用的账号");
    println!(
        "== 账号 {} ({}) sandbox={} ==\n",
        acc.appid, acc.name, acc.sandbox
    );

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let disp: Arc<OnceLock<Dispatcher>> = Arc::new(OnceLock::new());
    let bot = BotClient::new(acc, http.clone(), db.clone(), disp);
    let base = bot.api_base();

    // ---------- A. 全局接口 ----------
    println!("-- A. 全局 --");
    let token = match bot.access_token().await {
        Ok(t) => {
            unsafe { PASS += 1 };
            println!("PASS  {:<28} {}…", "access_token", &t[..24]);
            t
        }
        Err(e) => {
            unsafe { FAIL += 1 };
            println!("FAIL  {:<28} {e:#}", "access_token");
            println!(
                "\n== 结果：PASS={} FAIL={} SKIP={} ==",
                unsafe { PASS },
                unsafe { FAIL },
                unsafe { SKIP }
            );
            anyhow::bail!("拿不到 token，后续无意义");
        }
    };

    report("bot_info", &api::bot_info(&http, &token, base).await);
    report("menu_get", &api::menu_get(&http, &token, base).await);
    for scope in ["c2c", "group", "channel", "dm"] {
        report(
            &format!("panels_list({scope})"),
            &api::panels_list(&http, &token, base, scope, "", 20).await,
        );
    }
    report(
        "material_list",
        &api::material_list(&http, &token, bot.appid, "").await,
    );
    report(
        "generate_url_link",
        &api::generate_url_link(&http, &token, base, "api_probe").await,
    );
    report(
        "join_strategy_list",
        &api::join_strategy_list(&http, &token, base, "", 20).await,
    );

    // ---------- B. 群域 ----------
    println!("\n-- B. 群域 --");
    if group.is_empty() {
        skip("群域全部", "未传 group_openid（@一下机器人让日志拿到）");
    } else {
        report(
            "group_info",
            &api::group_info(&http, &token, base, &group).await,
        );
        report(
            "group_bot_state",
            &api::group_bot_state(&http, &token, base, &group).await,
        );
        let members = api::group_members(&http, &token, base, &group, "").await;
        report("group_members", &members);
        let member_openid = if !member.is_empty() {
            Some(member.clone())
        } else {
            members
                .as_ref()
                .ok()
                .and_then(|v| v["members"].as_array().cloned())
                .and_then(|m| m.first().cloned())
                .and_then(|m| m["member_openid"].as_str().map(str::to_string))
        };
        match &member_openid {
            Some(mo) => {
                report(
                    "group_member",
                    &api::group_member(&http, &token, base, &group, mo).await,
                );
            }
            None => skip("group_member", "members 列表为空或取不到 openid"),
        }
        report(
            "group_blacklist",
            &api::group_blacklist(&http, &token, base, &group, "", 20).await,
        );
        report(
            "group_mute_setting",
            &api::group_mute_setting(&http, &token, base, &group).await,
        );
        report(
            "group_join_requests",
            &api::group_join_requests(&http, &token, base, &group, "", 20).await,
        );

        // 消息发送 → 撤回 闭环
        let opts = api::SendOpts::default();
        let sent = api::send_text(
            &http,
            &token,
            base,
            0,
            &group,
            "[自检] api_probe 文本消息",
            opts,
        )
        .await;
        report("send_text(group)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(group)",
                    &api::recall(&http, &token, base, 0, &group, mid).await,
                );
            } else {
                skip("recall(group)", "发送响应无 id");
            }
        } else {
            skip("recall(group)", "send_text 失败");
        }

        let sent = api::send_markdown(
            &http,
            &token,
            base,
            0,
            &group,
            "**api_probe 自检** markdown",
            api::SendOpts::default(),
        )
        .await;
        report("send_markdown(group)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(markdown)",
                    &api::recall(&http, &token, base, 0, &group, mid).await,
                );
            }
        }

        let card_body = json!({
            "msg_type": 8,
            "card": {"type":"tuwen","content":{
                "title":"api_probe 自检卡片",
                "description":"msg_type=8 图文卡片",
                "url":"https://bot.q.qq.com",
                "pic_url":"https://qzonestyle.gtimg.cn/qzone/qzact/act/external/tiqq/logo.png"
            }}
        });
        let sent = api::post_send(&http, &token, base, 0, &group, &card_body).await;
        report("send_card(group)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(card)",
                    &api::recall(&http, &token, base, 0, &group, mid).await,
                );
            }
        }

        // 本地分片上传：12MB 让 md5_10m 边界真实走到（>10002432 字节）
        let big = std::env::temp_dir().join("mashiro_probe_12m.bin");
        std::fs::write(&big, vec![0xABu8; 12 * 1024 * 1024])?;
        let sent = api::send_media_file(
            &http,
            &token,
            base,
            0,
            &group,
            4,
            &big,
            api::SendOpts::default(),
        )
        .await;
        report("send_media_file(12MB)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(file)",
                    &api::recall(&http, &token, base, 0, &group, mid).await,
                );
            }
        }
        let small = std::env::temp_dir().join("mashiro_probe.txt");
        std::fs::write(&small, b"mashiro api probe")?;
        report(
            "upload_file",
            &api::upload_file(&http, &token, base, 0, &group, 4, &small).await,
        );

        // 策略 create→delete 闭环（自清理）
        let st = api::join_strategy_create(
            &http,
            &token,
            base,
            &json!({ "group_openids": [group], "is_enable": "off", "remark": "api_probe 自检，随即删除" }),
        )
        .await;
        report("strategy_create", &st);
        if let Ok(v) = &st {
            if let Some(sid) = v["strategy_id"].as_str() {
                report_unit(
                    "strategy_delete",
                    &api::join_strategy_delete(&http, &token, base, sid).await,
                );
            } else {
                skip("strategy_delete", "create 响应无 strategy_id");
            }
        } else {
            skip("strategy_delete", "create 失败");
        }

        // 被动回复（真实 msg_id，60 分钟有效）
        if !msg_id.is_empty() {
            let sent = api::send_text(
                &http,
                &token,
                base,
                0,
                &group,
                "[自检] 被动回复",
                api::SendOpts {
                    msg_id: Some(msg_id.clone()),
                    msg_seq: Some(1),
                    ..Default::default()
                },
            )
            .await;
            report("send_text(passive)", &sent);
            if let Ok(v) = &sent {
                if let Some(mid) = v["id"].as_str() {
                    report_unit(
                        "recall(passive)",
                        &api::recall(&http, &token, base, 0, &group, mid).await,
                    );
                }
            }
        } else {
            skip("send_text(passive)", "未传 msg_id");
        }

        // 需真实成员的写操作一律跳过（不动真实用户）
        skip(
            "blacklist_op/mute/remove",
            "会对真实成员生效，不做破坏性测试",
        );
        skip("approve_join", "需要真实待审批入群申请");
    }

    // ---------- C. 单聊域 ----------
    println!("\n-- C. 单聊 C2C --");
    if user.is_empty() {
        skip("C2C 全部", "未传 user_openid（私聊一下机器人让日志拿到）");
    } else {
        report(
            "user_info",
            &api::user_info(&http, &token, base, &user).await,
        );
        let sent = api::send_text(
            &http,
            &token,
            base,
            2,
            &user,
            "[自检] api_probe 单聊文本",
            api::SendOpts::default(),
        )
        .await;
        report("send_text(c2c)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(c2c)",
                    &api::recall(&http, &token, base, 2, &user, mid).await,
                );
            }
        }
        let s1 = api::send_stream(
            &http,
            &token,
            base,
            &user,
            &json!({"input_state":1,"index":0,"content_type":"text","content_raw":"api_probe 流式块1","msg_id":c2c_msg_id,"msg_seq":1}),
        )
        .await;
        report("send_stream#1", &s1);
        if let Ok(v) = &s1 {
            if let Some(sid) = v["id"].as_str() {
                let s2 = api::send_stream(
                    &http, &token, base, &user,
                    &json!({"input_state":10,"index":1,"content_type":"text","content_raw":"api_probe 流式块2/结束","stream_msg_id":sid,"msg_id":c2c_msg_id,"msg_seq":2}),
                ).await;
                report("send_stream#2", &s2);
            } else {
                skip("send_stream#2", "首块响应无 id");
            }
        }
        let f = std::env::temp_dir().join("mashiro_probe.txt");
        if !f.exists() {
            std::fs::write(&f, b"mashiro api probe")?;
        }
        let sent = api::send_media_file(
            &http,
            &token,
            base,
            2,
            &user,
            4,
            &f,
            api::SendOpts::default(),
        )
        .await;
        report("send_media_file(c2c)", &sent);
        if let Ok(v) = &sent {
            if let Some(mid) = v["id"].as_str() {
                report_unit(
                    "recall(c2c file)",
                    &api::recall(&http, &token, base, 2, &user, mid).await,
                );
            }
        }
    }

    // ---------- D. 恒跳过 ----------
    println!("\n-- D. 需现场事件 --");
    skip(
        "interaction_respond",
        "需要实时 INTERACTION_CREATE 的 interaction_id",
    );
    skip(
        "send_media(URL)",
        "URL 转存依赖公网可达性，改用本地分片上传已覆盖 files 链路",
    );

    println!(
        "\n== 结果：PASS={} FAIL={} SKIP={} ==",
        unsafe { PASS },
        unsafe { FAIL },
        unsafe { SKIP }
    );
    Ok(())
}
