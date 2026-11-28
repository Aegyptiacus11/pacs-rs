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
            // modify since its migration: the admin role permits everything
            perms(&["read", "write", "remote", "delete", "modify", "users"])
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

#[tokio::test]
async fn clients_are_created_found_changed_and_removed() {
    for index in common::indexes().await {
        assert_eq!(index.clients().await.unwrap(), []);
        assert_eq!(index.client("nobody").await.unwrap(), None);

        let viewer = index
            .create_client("c1", "Viewer", "hash-1", "reader", "ada")
            .await
            .unwrap();
        assert!(viewer.created_ms > 0);
        assert_eq!(viewer.last_token_ms, None);
        let pipeline = index
            .create_client("c2", "Pipeline", "hash-2", "editor", "ada")
            .await
            .unwrap();
        assert_eq!(
            index.clients().await.unwrap(),
            [viewer.clone(), pipeline.clone()]
        );
        assert_eq!(index.client("c1").await.unwrap(), Some(viewer.clone()));
        let again = index
            .create_client("c1", "Other", "hash-3", "reader", "ada")
            .await;
        assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");

        let changed = index
            .update_client("c2", Some("Nightly pipeline"), Some("manager"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (changed.name.as_str(), changed.role.as_str()),
            ("Nightly pipeline", "manager")
        );
        // the secret and who made it stay
        assert_eq!(
            (changed.secret_hash.as_str(), changed.created_by.as_str()),
            ("hash-2", "ada")
        );
        // nothing to change leaves the client as it is
        assert_eq!(
            index.update_client("c2", None, None).await.unwrap(),
            Some(changed)
        );
        assert_eq!(
            index
                .update_client("nobody", Some("x"), None)
                .await
                .unwrap(),
            None
        );

        index.client_got_token("c1").await.unwrap();
        let used = index.client("c1").await.unwrap().unwrap();
        assert!(used.last_token_ms.is_some_and(|ms| ms >= viewer.created_ms));
        // a client that is gone is not an error: its token request was refused before
        index.client_got_token("nobody").await.unwrap();

        assert!(index.delete_client("c2").await.unwrap());
        assert!(!index.delete_client("c2").await.unwrap());
        assert_eq!(index.clients().await.unwrap(), [used]);
    }
}

#[tokio::test]
async fn a_role_a_client_has_stays() {
    for index in common::indexes().await {
        index
            .create_role("robot", "Robot", &["read".to_owned()])
            .await
            .unwrap();
        index
            .create_client("c1", "Viewer", "hash-1", "robot", "ada")
            .await
            .unwrap();
        let in_use = index.delete_role("robot").await;
        assert!(matches!(in_use, Err(Error::Conflict(_))), "{in_use:?}");
        assert!(index.delete_client("c1").await.unwrap());
        assert!(index.delete_role("robot").await.unwrap());
    }
}

fn through_provider<'a>(
    subject: &'a str,
    id: &'a str,
    login: &'a str,
) -> pacs_index::ProviderUser<'a> {
    pacs_index::ProviderUser {
        issuer: "https://idp.example",
        subject,
        id,
        login,
        password_hash: "unmatchable",
        role: "reader",
        link_by_login: false,
    }
}

#[tokio::test]
async fn a_provider_sign_in_makes_the_user_once_and_finds_them_after() {
    use pacs_index::ProviderLink::{Created, Known};
    for index in common::indexes().await {
        let (made, link) = index
            .provider_user(&through_provider("sub-1", "u1", "ada@example.com"))
            .await
            .unwrap();
        assert_eq!(link, Created);
        assert_eq!((made.id.as_str(), made.role.as_str()), ("u1", "reader"));
        // the subject is the key: a new id or login offered later changes nothing
        let (again, link) = index
            .provider_user(&through_provider("sub-1", "u9", "renamed@example.com"))
            .await
            .unwrap();
        assert_eq!((again, link), (made.clone(), Known));
        assert_eq!(
            index
                .user_by_identity("https://idp.example", "sub-1")
                .await
                .unwrap(),
            Some(made)
        );
        // the same subject at another provider is someone else
        assert_eq!(
            index
                .user_by_identity("https://other.example", "sub-1")
                .await
                .unwrap(),
            None
        );
        assert_eq!(index.user_count().await.unwrap(), 1);
    }
}

#[tokio::test]
async fn a_taken_login_is_linked_only_when_the_provider_vouches_for_it() {
    use pacs_index::ProviderLink::Linked;
    for index in common::indexes().await {
        index
            .create_user("u1", "ada@example.com", "hash", "admin")
            .await
            .unwrap();
        // not vouched for: refused, and nothing is linked
        let refused = index
            .provider_user(&through_provider("sub-1", "u2", "ada@example.com"))
            .await;
        assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
        assert_eq!(
            index
                .user_by_identity("https://idp.example", "sub-1")
                .await
                .unwrap(),
            None
        );
        // vouched for: the existing user, role kept
        let vouched = pacs_index::ProviderUser {
            link_by_login: true,
            ..through_provider("sub-1", "u2", "ada@example.com")
        };
        let (linked, link) = index.provider_user(&vouched).await.unwrap();
        assert_eq!(link, Linked);
        assert_eq!((linked.id.as_str(), linked.role.as_str()), ("u1", "admin"));
        // a second subject at the same provider cannot take the same account
        let other = pacs_index::ProviderUser {
            link_by_login: true,
            ..through_provider("sub-2", "u3", "ada@example.com")
        };
        let refused = index.provider_user(&other).await;
        assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
        // a deleted user takes their identity with them
        index
            .create_user("u4", "boss", "hash", "admin")
            .await
            .unwrap();
        assert!(index.delete_user("u1", "admin").await.unwrap());
        assert_eq!(
            index
                .user_by_identity("https://idp.example", "sub-1")
                .await
                .unwrap(),
            None
        );
    }
}
