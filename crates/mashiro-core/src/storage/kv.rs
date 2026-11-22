use rusqlite::{params, OptionalExtension};

use super::db::Database;

/// 插件私有 KV：sdk_api 层直接调这几个函数。
impl Database {
    pub fn kv_get(&self, plugin_id: &str, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn.lock().query_row(
            "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin_id, key],
            |r| r.get(0),
        )
        .optional()
    }

    pub fn kv_set(&self, plugin_id: &str, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO plugin_kv(plugin_id, key, value) VALUES(?1, ?2, ?3)
             ON CONFLICT(plugin_id, key) DO UPDATE SET value = excluded.value",
            params![plugin_id, key, value],
        )?;
        Ok(())
    }

    pub fn kv_del(&self, plugin_id: &str, key: &str) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin_id, key],
        )?;
        Ok(())
    }

    /// 列出插件全部 KV（MCP/调试查看用）。
    pub fn kv_list(&self, plugin_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT key, value FROM plugin_kv WHERE plugin_id = ?1 ORDER BY key")?;
        let rows = stmt
            .query_map(params![plugin_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// +1 并返回新值（值不是整数按 0 起）。
    pub fn kv_incr(&self, plugin_id: &str, key: &str) -> rusqlite::Result<i64> {
        let cur: i64 = self
            .kv_get(plugin_id, key)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            + 1;
        self.kv_set(plugin_id, key, &cur.to_string())?;
        Ok(cur)
    }
}
