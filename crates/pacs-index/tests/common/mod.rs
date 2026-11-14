//! The backends every index test runs on: SQLite in memory always, and Postgres when
//! `PACS_TEST_POSTGRES_URL` names a server, e.g. `postgres://postgres:test@127.0.0.1:15432`
//! (docs/building.md). Each Postgres index is a fresh database of its own.

use std::sync::atomic::{AtomicU32, Ordering};

use pacs_index::Index;

pub async fn indexes() -> Vec<Index> {
    let mut indexes = vec![Index::in_memory().await.unwrap()];
    if let Some(index) = postgres().await {
        indexes.push(index);
    }
    indexes
}

/// A fresh Postgres index, if a server is configured.
pub async fn postgres() -> Option<Index> {
    let server = std::env::var("PACS_TEST_POSTGRES_URL").ok()?;
    let server = server.trim_end_matches('/');
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "pacs_test_{}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    );
    sqlx::any::install_default_drivers();
    let mut admin =
        <sqlx::AnyConnection as sqlx::Connection>::connect(&format!("{server}/postgres"))
            .await
            .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut admin)
        .await
        .unwrap();
    Some(Index::connect(&format!("{server}/{name}")).await.unwrap())
}
