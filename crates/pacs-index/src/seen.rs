//! Rights per project: which studies an account may see.
//!
//! An account is a user or a client, named by a key of the server's choosing
//! (`user:<id>`, `client:<id>`). It sees every study, unless it is limited: then it sees
//! the studies of its projects only, and never a study that is in no project. An
//! [`Index`] value made by [`Index::limited_to`] reads within those bounds: every read of
//! the hierarchy leaves out what the account may not see, as if it were not in the index.
//!
//! A project is an account too, under the key of [`Index::project_account`]: it sees its
//! own studies and nothing else, with nothing to set. It is who a DICOM application is
//! when it calls a project's AE title, since DIMSE has no accounts of its own.

use std::sync::Arc;

use crate::{Error, Index};

/// The account a request is limited to: what a server leaves in the request's extensions
/// for an account that does not see everything, and what a handler reads to bound its
/// index with [`Index::limited_to`]. No such extension: no limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenBy(pub String);

impl Index {
    /// The same index, read as `account` sees it. Writes are not bounded by it; those who
    /// write for an account check first with what they can read.
    pub fn limited_to(&self, account: &str) -> Index {
        Index {
            account: Some(Arc::from(account)),
            ..self.clone()
        }
    }

    /// The key of the account that sees the studies of `project` and no other. A user's
    /// key starts with `user:` and a tool's with `client:`, so it is nobody else's.
    pub fn project_account(project: &str) -> String {
        format!("project:{project}")
    }

    /// The same index, whole.
    pub fn unlimited(&self) -> Index {
        Index {
            account: None,
            ..self.clone()
        }
    }

    /// The account this value reads for, when it is limited.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Whether the study is there and this value may read it.
    pub(crate) async fn sees_study(&self, study_instance_uid: &str) -> Result<bool, Error> {
        let Some(account) = self.account() else {
            // no bound: a read of an absent study finds nothing on its own
            return Ok(true);
        };
        let seen: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM studies st
              WHERE st.study_instance_uid = $1
                AND (st.project IN (SELECT project FROM account_projects WHERE account = $2)
                     OR ('project:' || st.project) = $2)",
        )
        .bind(study_instance_uid)
        .bind(account)
        .fetch_one(&self.pool)
        .await?;
        Ok(seen > 0)
    }

    /// The projects `account` is limited to, in name order; `None` when it sees everything.
    pub async fn account_projects(&self, account: &str) -> Result<Option<Vec<String>>, Error> {
        let limited: i64 =
            sqlx::query_scalar("SELECT count(*) FROM limited_accounts WHERE account = $1")
                .bind(account)
                .fetch_one(&self.pool)
                .await?;
        if limited == 0 {
            return Ok(None);
        }
        Ok(Some(
            sqlx::query_scalar(
                "SELECT project FROM account_projects WHERE account = $1 ORDER BY project",
            )
            .bind(account)
            .fetch_all(&self.pool)
            .await?,
        ))
    }

    /// Limit `account` to `projects`, or with `None` let it see everything again. An empty
    /// list is a limit too: the account then sees no study.
    pub async fn set_account_projects(
        &self,
        account: &str,
        projects: Option<&[String]>,
    ) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        for table in [
            "DELETE FROM account_projects WHERE account = $1",
            "DELETE FROM limited_accounts WHERE account = $1",
        ] {
            sqlx::query(table).bind(account).execute(&mut *tx).await?;
        }
        if let Some(projects) = projects {
            sqlx::query("INSERT INTO limited_accounts (account) VALUES ($1)")
                .bind(account)
                .execute(&mut *tx)
                .await?;
            for project in projects {
                sqlx::query(
                    "INSERT INTO account_projects (account, project) VALUES ($1, $2)
                     ON CONFLICT (account, project) DO NOTHING",
                )
                .bind(account)
                .bind(project)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}
