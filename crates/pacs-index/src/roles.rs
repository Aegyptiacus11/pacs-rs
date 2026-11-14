//! Roles: a name and the permissions it gives. The index keeps them as text; which
//! permissions exist, and what each allows, is the server's business.

use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

/// One role as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRole {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub created_ms: i64,
}

fn role(row: &AnyRow) -> Result<StoredRole, Error> {
    let permissions: String = row.try_get("permissions")?;
    Ok(StoredRole {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        permissions: permissions
            .split(',')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect(),
        created_ms: row.try_get("created_ms")?,
    })
}

impl Index {
    /// Every role, in the order they were made.
    pub async fn roles(&self) -> Result<Vec<StoredRole>, Error> {
        let rows = sqlx::query(
            "SELECT id, name, permissions, created_ms FROM roles ORDER BY created_ms, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(role).collect()
    }

    pub async fn role(&self, id: &str) -> Result<Option<StoredRole>, Error> {
        let row = sqlx::query("SELECT id, name, permissions, created_ms FROM roles WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(role).transpose()
    }

    /// Add a role. [`Error::Conflict`] when the id is taken.
    pub async fn create_role(
        &self,
        id: &str,
        name: &str,
        permissions: &[String],
    ) -> Result<StoredRole, Error> {
        let mut tx = self.begin_write().await?;
        let taken: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if taken.is_some() {
            return Err(Error::Conflict(format!("there is already a role {id}")));
        }
        let created = StoredRole {
            id: id.to_owned(),
            name: name.to_owned(),
            permissions: permissions.to_vec(),
            created_ms: now_ms(),
        };
        sqlx::query(
            "INSERT INTO roles (id, name, permissions, created_ms) VALUES ($1, $2, $3, $4)",
        )
        .bind(&created.id)
        .bind(&created.name)
        .bind(created.permissions.join(","))
        .bind(created.created_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Change a role's name, its permissions, or both; the role after the change, or `None`
    /// when there is no such role.
    pub async fn update_role(
        &self,
        id: &str,
        name: Option<&str>,
        permissions: Option<&[String]>,
    ) -> Result<Option<StoredRole>, Error> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query("SELECT id, name, permissions, created_ms FROM roles WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(before) = row.as_ref().map(role).transpose()? else {
            return Ok(None);
        };
        let after = StoredRole {
            name: name.map_or(before.name.clone(), str::to_owned),
            permissions: permissions.map_or(before.permissions.clone(), <[String]>::to_vec),
            ..before
        };
        sqlx::query("UPDATE roles SET name = $1, permissions = $2 WHERE id = $3")
            .bind(&after.name)
            .bind(after.permissions.join(","))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(after))
    }

    /// Remove a role; whether there was one. [`Error::Conflict`] while a user has it.
    pub async fn delete_role(&self, id: &str) -> Result<bool, Error> {
        let mut tx = self.begin_write().await?;
        let holders: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        if holders > 0 {
            return Err(Error::Conflict(format!(
                "{holders} user(s) have this role: give them another one first"
            )));
        }
        let done = sqlx::query("DELETE FROM roles WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }
}
