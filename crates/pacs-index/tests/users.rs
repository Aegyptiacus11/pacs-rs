//! The users table, on every backend.

mod common;

use pacs_index::{Error, UserChange};

#[tokio::test]
async fn users_are_created_found_changed_and_removed() {
    for index in common::indexes().await {
        assert_eq!(index.user_count().await.unwrap(), 0);
        assert_eq!(index.users().await.unwrap(), []);
        assert_eq!(index.user("nobody").await.unwrap(), None);
        assert_eq!(index.user_by_login("nobody").await.unwrap(), None);

        let ada = index
            .create_user("u1", "ada", "hash-1", "admin")
            .await
            .unwrap();
        assert!(ada.active && ada.created_ms > 0);
        let bob = index
            .create_user("u2", "bob", "hash-2", "reader")
            .await
            .unwrap();
        assert_eq!(index.user_count().await.unwrap(), 2);
        assert_eq!(index.users().await.unwrap(), [ada.clone(), bob.clone()]);
        assert_eq!(index.user("u2").await.unwrap(), Some(bob.clone()));
        assert_eq!(index.user_by_login("ada").await.unwrap(), Some(ada.clone()));
        // compared as given: the caller lower-cases
        assert_eq!(index.user_by_login("Ada").await.unwrap(), None);

        let change = UserChange {
            password_hash: Some("hash-3".into()),
            role: Some("editor".into()),
            active: Some(false),
        };
        let changed = index
            .update_user("u2", &change, "admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                changed.password_hash.as_str(),
                changed.role.as_str(),
                changed.active
            ),
            ("hash-3", "editor", false)
        );
        assert_eq!(index.user("u2").await.unwrap(), Some(changed.clone()));
        // nothing to change leaves the user as it is
        assert_eq!(
            index
                .update_user("u2", &UserChange::default(), "admin")
                .await
                .unwrap(),
            Some(changed)
        );
        assert_eq!(
            index.update_user("nobody", &change, "admin").await.unwrap(),
            None
        );

        assert!(index.delete_user("u2", "admin").await.unwrap());
        assert!(!index.delete_user("u2", "admin").await.unwrap());
        assert_eq!(index.users().await.unwrap(), [ada]);
    }
}

#[tokio::test]
async fn a_login_is_taken_once() {
    for index in common::indexes().await {
        index.create_user("u1", "ada", "h", "admin").await.unwrap();
        let again = index.create_user("u2", "ada", "h", "reader").await;
        assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
        assert_eq!(index.user_count().await.unwrap(), 1);
    }
}

#[tokio::test]
async fn the_last_active_administrator_stays() {
    for index in common::indexes().await {
        index.create_user("u1", "ada", "h", "admin").await.unwrap();
        index.create_user("u2", "bob", "h", "reader").await.unwrap();
        let conflict = |r: Result<_, Error>| matches!(r, Err(Error::Conflict(_)));

        let demote = UserChange {
            role: Some("reader".into()),
            ..UserChange::default()
        };
        let deactivate = UserChange {
            active: Some(false),
            ..UserChange::default()
        };
        assert!(conflict(
            index.update_user("u1", &demote, "admin").await.map(|_| ())
        ));
        assert!(conflict(
            index
                .update_user("u1", &deactivate, "admin")
                .await
                .map(|_| ())
        ));
        assert!(conflict(index.delete_user("u1", "admin").await.map(|_| ())));
        let ada = index.user("u1").await.unwrap().unwrap();
        assert_eq!((ada.role.as_str(), ada.active), ("admin", true));
        // a new password is no loss of the role
        let password = UserChange {
            password_hash: Some("h2".into()),
            ..UserChange::default()
        };
        assert!(
            index
                .update_user("u1", &password, "admin")
                .await
                .unwrap()
                .is_some()
        );

        // with a second administrator, the first may go; a deactivated one does not count
        let promote = UserChange {
            role: Some("admin".into()),
            ..UserChange::default()
        };
        index.update_user("u2", &promote, "admin").await.unwrap();
        index.update_user("u2", &deactivate, "admin").await.unwrap();
        assert!(conflict(index.delete_user("u1", "admin").await.map(|_| ())));
        let activate = UserChange {
            active: Some(true),
            ..UserChange::default()
        };
        index.update_user("u2", &activate, "admin").await.unwrap();
        assert!(index.delete_user("u1", "admin").await.unwrap());
        assert!(conflict(index.delete_user("u2", "admin").await.map(|_| ())));
    }
}

#[tokio::test]
async fn the_refresh_tables_are_there() {
    for index in common::indexes().await {
        for table in ["auth_refresh_families", "auth_refresh_tokens"] {
            let rows: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                    .fetch_one(index.pool())
                    .await
                    .unwrap();
            assert_eq!(rows, 0);
        }
    }
}

#[tokio::test]
async fn roles_start_as_four_and_are_made_changed_and_removed() {
    for index in common::indexes().await {
        let perms = |list: &[&str]| list.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        let roles = index.roles().await.unwrap();
        let ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["reader", "editor", "manager", "admin"]);
        assert_eq!(roles[0].permissions, perms(&["read"]));
        assert_eq!(
            roles[3].permissions,
            perms(&["read", "write", "remote", "delete", "users"])
        );
        assert_eq!(index.role("nobody").await.unwrap(), None);

        let student = index
            .create_role("student", "Student", &perms(&["read"]))
            .await
            .unwrap();
        assert_eq!(index.role("student").await.unwrap(), Some(student.clone()));
        assert_eq!(index.roles().await.unwrap().last(), Some(&student));
        let again = index.create_role("student", "Other", &[]).await;
        assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");

        let renamed = index
            .update_role("student", Some("Students"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (renamed.name.as_str(), &renamed.permissions),
            ("Students", &perms(&["read"]))
        );
        let widened = index
            .update_role("student", None, Some(&perms(&["read", "write"])))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (widened.name.as_str(), &widened.permissions),
            ("Students", &perms(&["read", "write"]))
        );
        let emptied = index
            .update_role("student", None, Some(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(emptied.permissions, perms(&[]));
        assert_eq!(index.role("student").await.unwrap(), Some(emptied));
        assert_eq!(
            index.update_role("nobody", Some("x"), None).await.unwrap(),
            None
        );

        // a role someone has stays
        index
            .create_user("u1", "ada", "h", "student")
            .await
            .unwrap();
        let in_use = index.delete_role("student").await;
        assert!(matches!(in_use, Err(Error::Conflict(_))), "{in_use:?}");
        index.delete_user("u1", "admin").await.unwrap();
        assert!(index.delete_role("student").await.unwrap());
        assert!(!index.delete_role("student").await.unwrap());
    }
}
