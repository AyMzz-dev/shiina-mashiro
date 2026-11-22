use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use super::models::{Account, LogRow, PluginRow, TimerRow};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts(
    appid   INTEGER PRIMARY KEY,
    secret  TEXT NOT NULL DEFAULT '',
    name    TEXT NOT NULL DEFAULT '',
    avatar  TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS plugins(
    uuid    TEXT PRIMARY KEY,
    dir     TEXT NOT NULL,
    runtime TEXT NOT NULL,
    name    TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 0,
    config  TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS timers(
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id   TEXT NOT NULL,
    remark      TEXT NOT NULL DEFAULT '',
    next_run    INTEGER NOT NULL,
    repeat_secs INTEGER NOT NULL DEFAULT 0,
    enabled     INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS plugin_kv(
    plugin_id TEXT NOT NULL,
    key       TEXT NOT NULL,
    value     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(plugin_id, key)
);
CREATE TABLE IF NOT EXISTS uid_map(
    appid    INTEGER NOT NULL,
    openid   TEXT NOT NULL,
    short_id INTEGER NOT NULL,
    PRIMARY KEY(appid, openid)
);
CREATE TABLE IF NOT EXISTS logs(
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts       INTEGER NOT NULL,
    appid    INTEGER NOT NULL DEFAULT 0,
    kind     TEXT NOT NULL DEFAULT '',
    group_id TEXT NOT NULL DEFAULT '',
    user     TEXT NOT NULL DEFAULT '',
    content  TEXT NOT NULL DEFAULT '',
    raw      TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs(ts);
CREATE TABLE IF NOT EXISTS settings(
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS plugin_stats(
    plugin_id TEXT NOT NULL,
    ymd       TEXT NOT NULL,
    n         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(plugin_id, ymd)
);
"#;

/// 数据目录：exe 旁的 data/（开发期 exe 在 target/debug 下，同样规则）。
pub fn default_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data")
}

/// SQLite 单文件存储。rusqlite 非线程安全 → 一把 Mutex 串行化即可（机器人量级无压力）。
pub struct Database {
    pub(crate) conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        // 老库增量补列：PRAGMA table_info 查缺 → ALTER TABLE ADD COLUMN
        {
            let cols: std::collections::HashSet<String> = conn
                .prepare("PRAGMA table_info(accounts)")?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<_>>()?;
            for (col, ddl) in [
                (
                    "mode",
                    "ALTER TABLE accounts ADD COLUMN mode INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "sandbox",
                    "ALTER TABLE accounts ADD COLUMN sandbox INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "md_group",
                    "ALTER TABLE accounts ADD COLUMN md_group INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "md_guild",
                    "ALTER TABLE accounts ADD COLUMN md_guild INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "md_guild_tpl",
                    "ALTER TABLE accounts ADD COLUMN md_guild_tpl INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "ws_intents",
                    "ALTER TABLE accounts ADD COLUMN ws_intents INTEGER NOT NULL DEFAULT 0",
                ),
                (
                    "ws_url",
                    "ALTER TABLE accounts ADD COLUMN ws_url TEXT NOT NULL DEFAULT ''",
                ),
            ] {
                if !cols.contains(col) {
                    conn.execute_batch(ddl)?;
                }
            }
        }
        {
            let cols: std::collections::HashSet<String> = conn
                .prepare("PRAGMA table_info(logs)")?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<_>>()?;
            for (col, ddl) in [(
                "raw",
                "ALTER TABLE logs ADD COLUMN raw TEXT NOT NULL DEFAULT ''",
            )] {
                if !cols.contains(col) {
                    conn.execute_batch(ddl)?;
                }
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 默认位置：<程序运行目录>/data/mashiro.db。
    /// 首次迁移：新库不存在而旧的 %APPDATA%/mashiro/mashiro.db 还在 → 复制过来（不删旧文件）。
    pub fn open_default() -> anyhow::Result<Self> {
        let dir = default_dir();
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("mashiro.db");
        if !db_path.exists() {
            if let Some(old) = dirs_next::data_dir().map(|d| d.join("mashiro").join("mashiro.db")) {
                if old.exists() {
                    let _ = std::fs::copy(&old, &db_path);
                }
            }
        }
        Ok(Self::open(db_path)?)
    }

    // ---------- accounts ----------

    const ACCOUNT_COLS: &'static str =
        "appid, secret, name, avatar, enabled, mode, sandbox, md_group, md_guild, md_guild_tpl, ws_intents, ws_url";

    fn row_to_account(r: &rusqlite::Row) -> rusqlite::Result<Account> {
        Ok(Account {
            appid: r.get(0)?,
            secret: super::secret::reveal(&r.get::<_, String>(1)?),
            name: r.get(2)?,
            avatar: r.get(3)?,
            enabled: r.get::<_, i64>(4)? != 0,
            mode: r.get(5)?,
            sandbox: r.get::<_, i64>(6)? != 0,
            md_group: r.get::<_, i64>(7)? != 0,
            md_guild: r.get::<_, i64>(8)? != 0,
            md_guild_tpl: r.get::<_, i64>(9)? != 0,
            ws_intents: r.get(10)?,
            ws_url: r.get(11)?,
        })
    }

    /// 新增/更新账号全量配置。secret 传空串 = 保留库内原值。
    pub fn save_account(&self, a: &Account) -> rusqlite::Result<()> {
        let sealed = super::secret::seal(&a.secret);
        self.conn.lock().execute(
            "INSERT INTO accounts(appid, secret, name, enabled, mode, sandbox,
                                  md_group, md_guild, md_guild_tpl, ws_intents, ws_url)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(appid) DO UPDATE SET
                secret       = CASE WHEN excluded.secret = '' THEN accounts.secret
                                    ELSE excluded.secret END,
                name         = excluded.name,
                enabled      = excluded.enabled,
                mode         = excluded.mode,
                sandbox      = excluded.sandbox,
                md_group     = excluded.md_group,
                md_guild     = excluded.md_guild,
                md_guild_tpl = excluded.md_guild_tpl,
                ws_intents   = excluded.ws_intents,
                ws_url       = excluded.ws_url",
            params![
                a.appid,
                sealed,
                a.name,
                a.enabled as i64,
                a.mode,
                a.sandbox as i64,
                a.md_group as i64,
                a.md_guild as i64,
                a.md_guild_tpl as i64,
                a.ws_intents,
                a.ws_url,
            ],
        )?;
        Ok(())
    }

    /// READY 后回填昵称/头像（只动这两个字段，不碰 secret/配置）。
    pub fn update_account_profile(
        &self,
        appid: i64,
        name: &str,
        avatar: &str,
    ) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "UPDATE accounts SET name = ?2, avatar = ?3 WHERE appid = ?1",
            params![appid, name, avatar],
        )?;
        Ok(())
    }

    pub fn remove_account(&self, appid: i64) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM accounts WHERE appid = ?1", params![appid])?;
        Ok(())
    }

    pub fn get_account(&self, appid: i64) -> rusqlite::Result<Option<Account>> {
        let conn = self.conn.lock();
        conn.query_row(
            &format!(
                "SELECT {} FROM accounts WHERE appid = ?1",
                Self::ACCOUNT_COLS
            ),
            params![appid],
            Self::row_to_account,
        )
        .optional()
    }

    pub fn list_accounts(&self) -> rusqlite::Result<Vec<Account>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM accounts ORDER BY appid",
            Self::ACCOUNT_COLS
        ))?;
        let out = stmt.query_map([], Self::row_to_account)?.collect();
        out
    }

    // ---------- plugins ----------

    pub fn upsert_plugin(&self, p: &PluginRow) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO plugins(uuid, dir, runtime, name, enabled, config) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(uuid) DO UPDATE SET dir=excluded.dir, runtime=excluded.runtime,
             name=excluded.name, config=excluded.config",
            params![p.uuid, p.dir, p.runtime, p.name, p.enabled as i64, p.config],
        )?;
        Ok(())
    }

    pub fn list_plugins(&self) -> rusqlite::Result<Vec<PluginRow>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT uuid, dir, runtime, name, enabled, config FROM plugins")?;
        let out = stmt
            .query_map([], |r| {
                Ok(PluginRow {
                    uuid: r.get(0)?,
                    dir: r.get(1)?,
                    runtime: r.get(2)?,
                    name: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                    config: r.get(5)?,
                })
            })?
            .collect();
        out
    }

    pub fn set_plugin_enabled(&self, uuid: &str, enabled: bool) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "UPDATE plugins SET enabled = ?1 WHERE uuid = ?2",
            params![enabled as i64, uuid],
        )?;
        Ok(())
    }

    pub fn remove_plugin(&self, uuid: &str) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM plugins WHERE uuid = ?1", params![uuid])?;
        Ok(())
    }

    /// 覆盖写 plugins.config 的 JSON（合并由调用方负责）。
    pub fn update_plugin_config(&self, uuid: &str, config: &str) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "UPDATE plugins SET config = ?1 WHERE uuid = ?2",
            params![config, uuid],
        )?;
        Ok(())
    }

    /// 插件账号授权：config.accounts 为空/缺省 = 允许所有账号；否则只放行表内 appid。
    pub fn plugin_allowed(&self, uuid: &str, appid: i64) -> bool {
        let config: String = match self
            .conn
            .lock()
            .query_row(
                "SELECT config FROM plugins WHERE uuid = ?1",
                params![uuid],
                |r| r.get(0),
            )
            .optional()
        {
            Ok(Some(c)) => c,
            _ => return false,
        };
        let v: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();
        match v["accounts"].as_array() {
            Some(a) if !a.is_empty() => a.iter().any(|x| x.as_i64() == Some(appid)),
            _ => true,
        }
    }

    // ---------- timers ----------

    pub fn insert_timer(&self, t: &TimerRow) -> rusqlite::Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO timers(plugin_id, remark, next_run, repeat_secs, enabled)
             VALUES(?1,?2,?3,?4,1)",
            params![t.plugin_id, t.remark, t.next_run, t.repeat_secs],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn due_timers(&self, now: i64) -> rusqlite::Result<Vec<TimerRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, plugin_id, remark, next_run, repeat_secs, enabled FROM timers
             WHERE enabled = 1 AND next_run <= ?1",
        )?;
        let out = stmt
            .query_map(params![now], |r| {
                Ok(TimerRow {
                    id: r.get(0)?,
                    plugin_id: r.get(1)?,
                    remark: r.get(2)?,
                    next_run: r.get(3)?,
                    repeat_secs: r.get(4)?,
                    enabled: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect();
        out
    }

    pub fn bump_timer(&self, id: i64, next_run: i64) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "UPDATE timers SET next_run = ?1 WHERE id = ?2",
            params![next_run, id],
        )?;
        Ok(())
    }

    pub fn delete_timer(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM timers WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 全部定时任务（任务管理页展示用）。
    pub fn list_timers(&self) -> rusqlite::Result<Vec<TimerRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, plugin_id, remark, next_run, repeat_secs, enabled FROM timers
             ORDER BY next_run",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TimerRow {
                    id: r.get(0)?,
                    plugin_id: r.get(1)?,
                    remark: r.get(2)?,
                    next_run: r.get(3)?,
                    repeat_secs: r.get(4)?,
                    enabled: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn set_timer_enabled(&self, id: i64, enabled: bool) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "UPDATE timers SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        Ok(())
    }

    // ---------- uid_map：用户整数短 ID ----------

    /// openid → 短 ID，没有就分配一个新的（max+1，从 1 起）。
    pub fn short_id(&self, appid: i64, openid: &str) -> rusqlite::Result<i32> {
        let conn = self.conn.lock();
        if let Some(id) = conn
            .query_row(
                "SELECT short_id FROM uid_map WHERE appid=?1 AND openid=?2",
                params![appid, openid],
                |r| r.get::<_, i32>(0),
            )
            .optional()?
        {
            return Ok(id);
        }
        let next: i32 = conn.query_row(
            "SELECT COALESCE(MAX(short_id), 0) + 1 FROM uid_map WHERE appid=?1",
            params![appid],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO uid_map(appid, openid, short_id) VALUES(?1,?2,?3)",
            params![appid, openid, next],
        )?;
        Ok(next)
    }

    /// 短 ID → openid 反映射（插件 get_openid 用）。
    pub fn openid_of(&self, appid: i64, short_id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .lock()
            .query_row(
                "SELECT openid FROM uid_map WHERE appid=?1 AND short_id=?2",
                params![appid, short_id],
                |r| r.get(0),
            )
            .optional()
    }

    // ---------- settings：框架级 KV 配置（解释器路径/日志保留天数等） ----------

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<String> {
        self.conn
            .lock()
            .query_row("SELECT v FROM settings WHERE k = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .map(|v| v.unwrap_or_default())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO settings(k, v) VALUES(?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![key, value],
        )?;
        Ok(())
    }

    // ---------- stats ----------

    pub fn count_logs_since(&self, ts: i64) -> rusqlite::Result<i64> {
        self.conn.lock().query_row(
            "SELECT COUNT(*) FROM logs WHERE ts >= ?1",
            params![ts],
            |r| r.get(0),
        )
    }

    // ---------- logs ----------

    pub fn insert_log(&self, row: &LogRow) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO logs(ts, appid, kind, group_id, user, content, raw) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.ts, row.appid, row.kind, row.group_id, row.user, row.content, row.raw],
        )?;
        Ok(())
    }

    /// kinds: 空 = 全部类型；appid: 0/None = 全部账号。
    pub fn list_logs(
        &self,
        kinds: &[String],
        appid: i64,
        limit: i64,
    ) -> rusqlite::Result<Vec<LogRow>> {
        let conn = self.conn.lock();
        let mut sql =
            "SELECT id, ts, appid, kind, group_id, user, content, raw FROM logs".to_string();
        let mut conds: Vec<String> = Vec::new();
        let mut vals: Vec<rusqlite::types::Value> = Vec::new();
        if !kinds.is_empty() {
            conds.push(format!(
                "kind IN ({})",
                kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",")
            ));
            vals.extend(kinds.iter().map(|k| k.clone().into()));
        }
        if appid > 0 {
            conds.push("appid = ?".into());
            vals.push(appid.into());
        }
        if !conds.is_empty() {
            sql.push_str(&format!(" WHERE {}", conds.join(" AND ")));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        vals.push(limit.into());
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(vals), |r| {
                Ok(LogRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    appid: r.get(2)?,
                    kind: r.get(3)?,
                    group_id: r.get(4)?,
                    user: r.get(5)?,
                    content: r.get(6)?,
                    raw: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// 聊天室会话列表：消息类日志按 (appid, group_id) 聚合，取最新一条做预览。
    /// Send 日志与对应接收消息共享同一 group_id，天然并入同一会话。
    pub fn list_conversations(
        &self,
    ) -> rusqlite::Result<Vec<crate::storage::models::Conversation>> {
        use crate::storage::models::Conversation;
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT l.appid, l.group_id, l.kind, l.content, l.ts, c.cnt
             FROM logs l
             JOIN (SELECT appid, group_id, MAX(id) mid, COUNT(*) cnt FROM logs
                   WHERE kind IN ('Group','Guild','C2C','GuildDm','Send')
                   GROUP BY appid, group_id) c
             ON l.id = c.mid
             ORDER BY l.ts DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Conversation {
                    appid: r.get(0)?,
                    group_id: r.get(1)?,
                    kind: r.get(2)?,
                    last_content: r.get(3)?,
                    last_ts: r.get(4)?,
                    count: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// 会话消息流：按 id < before_id 倒序分页（前端反转后展示）。
    pub fn chat_history(
        &self,
        appid: i64,
        group_id: &str,
        before_id: i64,
        limit: i64,
    ) -> rusqlite::Result<Vec<LogRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, ts, appid, kind, group_id, user, content, raw FROM logs
             WHERE appid = ?1 AND group_id = ?2
               AND kind IN ('Group','Guild','C2C','GuildDm','Send')
               AND (?3 <= 0 OR id < ?3)
             ORDER BY id DESC LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(params![appid, group_id, before_id, limit], |r| {
                Ok(LogRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    appid: r.get(2)?,
                    kind: r.get(3)?,
                    group_id: r.get(4)?,
                    user: r.get(5)?,
                    content: r.get(6)?,
                    raw: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// 清空日志表。
    pub fn clear_logs(&self) -> rusqlite::Result<()> {
        self.conn.lock().execute("DELETE FROM logs", [])?;
        Ok(())
    }

    /// 删除单条日志。
    pub fn delete_log(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM logs WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 插件消息计数 +1（当日）。
    pub fn bump_plugin_stat(&self, plugin_id: &str) -> rusqlite::Result<()> {
        let ymd = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.conn.lock().execute(
            "INSERT INTO plugin_stats(plugin_id, ymd, n) VALUES(?1,?2,1)
             ON CONFLICT(plugin_id, ymd) DO UPDATE SET n = n + 1",
            params![plugin_id, ymd],
        )?;
        Ok(())
    }

    /// 今日插件消息 TOP N：(name, count)。
    pub fn top_plugins(&self, limit: i64) -> rusqlite::Result<Vec<(String, i64)>> {
        let ymd = chrono::Local::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT p.name, s.n FROM plugin_stats s
             LEFT JOIN plugins p ON p.uuid = s.plugin_id
             WHERE s.ymd = ?1 ORDER BY s.n DESC LIMIT ?2",
        )?;
        let out = stmt
            .query_map(params![ymd, limit], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, i64>(1)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(out)
    }

    /// 今日小时级收发统计：返回 [(hour, recv, send); <=24]。
    /// recv = Group/Guild/C2C/GuildDm 事件；send = Send。
    pub fn hourly_msgs(&self, day_start_ms: i64) -> rusqlite::Result<Vec<(u32, i64, i64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT CAST(strftime('%H', ts/1000, 'unixepoch', 'localtime') AS INTEGER) h,
                    SUM(CASE WHEN kind IN ('Group','Guild','C2C','GuildDm') THEN 1 ELSE 0 END) recv,
                    SUM(CASE WHEN kind = 'Send' THEN 1 ELSE 0 END) send
             FROM logs WHERE ts >= ?1 GROUP BY h ORDER BY h",
        )?;
        let out = stmt
            .query_map(params![day_start_ms], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(out)
    }

    /// 清理 ts 之前的日志（日志保留天数策略用）。
    pub fn delete_logs_before(&self, ts: i64) -> rusqlite::Result<usize> {
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM logs WHERE ts < ?1", params![ts])?;
        Ok(n)
    }
}
