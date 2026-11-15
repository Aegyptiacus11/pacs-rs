//! Clients: programs that connect without a person. The index keeps each one's name, the
//! hash of its secret and its role; what a secret is, and how it is checked, is the server's
//! business.

use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

/// One client as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClient {
    pub id: String,
    pub name: String,
    pub secret_hash: String,
    /// The id of its role.
    pub role: String,
    /// The login of the user who made it.
    pub created_by: String,
    pub created_ms: i64,
    /// When it last got an access token.
    pub last_token_ms: Option<i64>,
}

fn client(row: &AnyRow) -> Result<StoredClient, Error> {
    Ok(StoredClient {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        secret_hash: row.try_get("secret_hash")?,
        role: row.try_get("role")?,
        created_by: row.try_get("created_by")?,
        created_ms: row.try_get("created_ms")?,
        last_token_ms: row.try_get("last_token_ms")?,
    })
}

impl Index {
    /// Every client, in the order they were made.
    pub async fn clients(&self) -> Result<Vec<StoredClient>, Error> {
        let rows = sqlx::query(
            "SELECT id, name, secret_hash, role, created_by, created_ms, last_token_ms \
             FROM clients ORDER BY created_ms, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(client).collect()
    }

    pub async fn client(&self, id: &str) -> Result<Option<StoredClient>, Error> {
        let row = sqlx::query(
            "SELECT id, name, secret_hash, role, created_by, created_ms, last_token_ms \
             FROM clients WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(client).transpose()
    }

    /// Add a client. [`Error::Conflict`] when the id is taken.
    pub async fn create_client(
        &self,
        id: &str,
        name: &str,
        secret_hash: &str,
        role: &str,
        created_by: &str,
    ) -> Result<StoredClient, Error> {
        let mut tx = self.begin_write().await?;
        let taken: Option<String> = sqlx::query_scalar("SELECT id FROM clients WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if taken.is_some() {
            return Err(Error::Conflict(format!("there is already a client {id}")));
        }
        let created = StoredClient {
            id: id.to_owned(),
            name: name.to_owned(),
            secret_hash: secret_hash.to_owned(),
            role: role.to_owned(),
            created_by: created_by.to_owned(),
            created_ms: now_ms(),
            last_token_ms: None,
        };
        sqlx::query(
            "INSERT INTO clients (id, name, secret_hash, role, created_by, created_ms) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&created.id)
        .bind(&created.name)
        .bind(&created.secret_hash)
        .bind(&created.role)
        .bind(&created.created_by)
        .bind(created.created_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Change a client's name, its role, or both; the client after the change, or `None` when
    /// there is no such client.
    pub async fn update_client(
        &self,
        id: &str,
        name: Option<&str>,
        role: Option<&str>,
    ) -> Result<Option<StoredClient>, Error> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT id, name, secret_hash, role, created_by, created_ms, last_token_ms \
             FROM clients WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(before) = row.as_ref().map(client).transpose()? else {
            return Ok(None);
        };
        let after = StoredClient {
            name: name.map_or(before.name.clone(), str::to_owned),
            role: role.map_or(before.role.clone(), str::to_owned),
            ..before
        };
        sqlx::query("UPDATE clients SET name = $1, role = $2 WHERE id = $3")
            .bind(&after.name)
            .bind(&after.role)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(after))
    }

    /// Note that a client got an access token now.
    pub async fn client_got_token(&self, id: &str) -> Result<(), Error> {
        sqlx::query("UPDATE clients SET last_token_ms = $1 WHERE id = $2")
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove a client; whether there was one.
    pub async fn delete_client(&self, id: &str) -> Result<bool, Error> {
        let done = sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
