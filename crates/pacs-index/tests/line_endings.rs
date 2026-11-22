//! An index whose migrations were applied by a build with CRLF copies of them (the Windows
//! builds of 0.12.0 and before), opened by a build with LF ones: the same migrations, taken
//! as applied. The failure this reproduces: "migration 1 was previously applied but has been
//! modified", on Windows CI, opening the index 0.5.2 wrote on Linux.

use pacs_index::Index;
use sha2::Digest;

fn sha384(text: &str) -> Vec<u8> {
    sha2::Sha384::digest(text.as_bytes()).to_vec()
}

/// The migrations of this build, by version, with their text.
fn migrations() -> Vec<(i64, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    let mut found: Vec<(i64, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let version = name.split('_').next().unwrap().parse().unwrap();
            (version, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    found
}

async fn set_checksums(path: &std::path::Path, of: impl Fn(&str) -> Vec<u8>) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    for (version, sql) in migrations() {
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(of(&sql))
            .bind(version)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
}

#[tokio::test]
async fn migrations_applied_with_other_line_endings_are_taken_as_applied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    drop(Index::open(&path).await.unwrap());
    // as a Windows build of 0.12.0 would have left them
    set_checksums(&path, |sql| {
        sha384(&sql.replace("\r\n", "\n").replace('\n', "\r\n"))
    })
    .await;
    let index = Index::open(&path)
        .await
        .expect("the same migrations, other line endings");
    assert_eq!(index.user_count().await.unwrap(), 0);
    drop(index);
    // and the other way: LF recorded, opened by any build
    set_checksums(&path, |sql| sha384(&sql.replace("\r\n", "\n"))).await;
    Index::open(&path).await.unwrap();
}

#[tokio::test]
async fn a_migration_that_really_changed_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    drop(Index::open(&path).await.unwrap());
    set_checksums(&path, |sql| sha384(&format!("{sql}-- changed\n"))).await;
    let error = match Index::open(&path).await {
        Ok(_) => panic!("a changed migration is refused"),
        Err(e) => e.to_string(),
    };
    assert!(
        error.contains("previously applied but has been modified"),
        "{error}"
    );
}
