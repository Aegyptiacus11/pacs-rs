//! The remote AEs added in the web interface, kept so that the server finds them again when it
//! starts; those of `PACS_DIMSE_REMOTES` stay in the settings. What each is checked against
//! (an AE title, a host, a port) is the server's business; the index keeps what it is given.

use sqlx::Row;

use crate::{Error, Index, now_ms};

/// One stored remote AE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRemoteAe {
    pub id: String,
    pub ae_title: String,
    pub host: String,
    pub port: u16,
    pub secure: bool,
    pub may_receive: bool,
}

impl Index {
    /// Every stored remote AE, by id.
    pub async fn remote_aes(&self) -> Result<Vec<StoredRemoteAe>, Error> {
        let rows = sqlx::query(
            "SELECT id, ae_title, host, port, secure, may_receive FROM remote_aes ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let port: i64 = row.try_get("port")?;
                Ok(StoredRemoteAe {
                    id: row.try_get("id")?,
                    ae_title: row.try_get("ae_title")?,
                    host: row.try_get("host")?,
                    port: u16::try_from(port).map_err(|_| {
                        Error::Database(sqlx::Error::Decode(
                            format!("remote AE port {port} is no TCP port").into(),
                        ))
                    })?,
                    secure: row.try_get::<i64, _>("secure")? != 0,
                    may_receive: row.try_get::<i64, _>("may_receive")? != 0,
                })
            })
            .collect()
    }

    /// Keeps `remote`, replacing the one with its id.
    pub async fn put_remote_ae(&self, remote: &StoredRemoteAe) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO remote_aes (id, ae_title, host, port, secure, may_receive, updated_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET ae_title = excluded.ae_title, host = excluded.host,
               port = excluded.port, secure = excluded.secure, may_receive = excluded.may_receive,
               updated_ms = excluded.updated_ms",
        )
        .bind(&remote.id)
        .bind(&remote.ae_title)
        .bind(&remote.host)
        .bind(i64::from(remote.port))
        .bind(i64::from(remote.secure))
        .bind(i64::from(remote.may_receive))
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes the remote AE `id`; whether there was one.
    pub async fn delete_remote_ae(&self, id: &str) -> Result<bool, Error> {
        let done = sqlx::query("DELETE FROM remote_aes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
