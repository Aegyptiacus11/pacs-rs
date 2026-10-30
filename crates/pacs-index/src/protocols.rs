//! The web interface's hanging protocols: opaque JSON documents by ID, newest first. The
//! index keeps them so that every browser on a server sees the same ones; what they say is
//! the interface's business.

use sqlx::Row;

use crate::{Error, Index, now_ms};

/// One stored protocol: its ID, its JSON text, when it was last saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProtocol {
    pub id: String,
    pub body: String,
    pub updated_ms: i64,
}

impl Index {
    /// Every protocol, the most recently saved first.
    pub async fn protocols(&self) -> Result<Vec<StoredProtocol>, Error> {
        let rows = sqlx::query(
            "SELECT id, body, updated_ms FROM ui_protocols ORDER BY updated_ms DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(StoredProtocol {
                    id: row.try_get("id")?,
                    body: row.try_get("body")?,
                    updated_ms: row.try_get("updated_ms")?,
                })
            })
            .collect()
    }

    /// Keep `body` as protocol `id`, replacing what was there.
    pub async fn put_protocol(&self, id: &str, body: &str) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO ui_protocols (id, body, updated_ms) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET body = excluded.body, updated_ms = excluded.updated_ms",
        )
        .bind(id)
        .bind(body)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove protocol `id`; whether there was one.
    pub async fn delete_protocol(&self, id: &str) -> Result<bool, Error> {
        let done = sqlx::query("DELETE FROM ui_protocols WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
