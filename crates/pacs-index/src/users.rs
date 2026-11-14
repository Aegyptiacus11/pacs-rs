//! The users who may sign in, when the server asks for a login. The index keeps who they
//! are; what a role allows, and how a password is hashed, is the server's business.

use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

/// One user as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUser {
    pub id: String,
    /// Lower case.
    pub login: String,
    pub password_hash: String,
    pub role: String,
    pub active: bool,
    pub created_ms: i64,
}

/// What to change about a user; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserChange {
    pub password_hash: Option<String>,
    pub role: Option<String>,
    pub active: Option<bool>,
}

fn user(row: &AnyRow) -> Result<StoredUser, Error> {
    Ok(StoredUser {
        id: row.try_get("id")?,
        login: row.try_get("login")?,
        password_hash: row.try_get("password_hash")?,
        role: row.try_get("role")?,
        active: row.try_get::<i64, _>("active")? != 0,
        created_ms: row.try_get("created_ms")?,
    })
}

const COLUMNS: &str = "id, login, password_hash, role, active, created_ms";

impl Index {
    /// The pool the index is on, for a store that keeps its own tables in the same database.
    pub fn pool(&self) -> &sqlx::AnyPool {
        &self.pool
    }

    /// Every user, by login.
    pub async fn users(&self) -> Result<Vec<StoredUser>, Error> {
        let rows = sqlx::query(select(COLUMNS, "ORDER BY login"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(user).collect()
    }

    pub async fn user(&self, id: &str) -> Result<Option<StoredUser>, Error> {
        let row = sqlx::query(select(COLUMNS, "WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(user).transpose()
    }

    /// The user who signs in as `login`, which is compared as given: lower-case it first.
    pub async fn user_by_login(&self, login: &str) -> Result<Option<StoredUser>, Error> {
        let row = sqlx::query(select(COLUMNS, "WHERE login = $1"))
            .bind(login)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(user).transpose()
    }

    pub async fn user_count(&self) -> Result<i64, Error> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Add an active user. [`Error::Conflict`] when the login is taken.
    pub async fn create_user(
        &self,
        id: &str,
        login: &str,
        password_hash: &str,
        role: &str,
    ) -> Result<StoredUser, Error> {
        let mut tx = self.begin_write().await?;
        let taken: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1")
            .bind(login)
            .fetch_optional(&mut *tx)
            .await?;
        if taken.is_some() {
            return Err(Error::Conflict(format!("the login {login} is taken")));
        }
        let created = StoredUser {
            id: id.to_owned(),
            login: login.to_owned(),
            password_hash: password_hash.to_owned(),
            role: role.to_owned(),
            active: true,
            created_ms: now_ms(),
        };
        sqlx::query(
            "INSERT INTO users (id, login, password_hash, role, active, created_ms)
             VALUES ($1, $2, $3, $4, 1, $5)",
        )
        .bind(&created.id)
        .bind(&created.login)
        .bind(&created.password_hash)
        .bind(&created.role)
        .bind(created.created_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Change user `id`; the user after the change, or `None` when there is no such user.
    /// [`Error::Conflict`] when the change would leave no active user with `kept_role`: the
    /// last administrator cannot be demoted or deactivated.
    pub async fn update_user(
        &self,
        id: &str,
        change: &UserChange,
        kept_role: &str,
    ) -> Result<Option<StoredUser>, Error> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(select(COLUMNS, "WHERE id = $1"))
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(before) = row.as_ref().map(user).transpose()? else {
            return Ok(None);
        };
        let after = StoredUser {
            password_hash: change
                .password_hash
                .clone()
                .unwrap_or(before.password_hash.clone()),
            role: change.role.clone().unwrap_or(before.role.clone()),
            active: change.active.unwrap_or(before.active),
            ..before.clone()
        };
        let held = |u: &StoredUser| u.active && u.role == kept_role;
        if held(&before) && !held(&after) {
            last_holder(&mut tx, kept_role).await?;
        }
        sqlx::query("UPDATE users SET password_hash = $1, role = $2, active = $3 WHERE id = $4")
            .bind(&after.password_hash)
            .bind(&after.role)
            .bind(i64::from(after.active))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(after))
    }

    /// Remove user `id`; whether there was one. [`Error::Conflict`] when they are the last
    /// active user with `kept_role`.
    pub async fn delete_user(&self, id: &str, kept_role: &str) -> Result<bool, Error> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(select(COLUMNS, "WHERE id = $1"))
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(before) = row.as_ref().map(user).transpose()? else {
            return Ok(false);
        };
        if before.active && before.role == kept_role {
            last_holder(&mut tx, kept_role).await?;
        }
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// Refuse when only one active user has `role`. Inside the write lock, so two changes at once
/// cannot each see the other's user as the one that remains.
async fn last_holder(
    tx: &mut sqlx::Transaction<'static, sqlx::Any>,
    role: &str,
) -> Result<(), Error> {
    let holders: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE active = 1 AND role = $1")
            .bind(role)
            .fetch_one(&mut **tx)
            .await?;
    if holders <= 1 {
        return Err(Error::Conflict(format!(
            "this is the last active user with the role {role}"
        )));
    }
    Ok(())
}

/// `SELECT <columns> FROM users <rest>`, for the handful of fixed queries above.
fn select(columns: &'static str, rest: &'static str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("SELECT {columns} FROM users {rest}"))
}
