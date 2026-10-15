//! Reversible local identity in Cursor's state database. Official logins stay owned by Cursor.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use cursor_engine::cursor::services::account::{
    is_local_token, local_token, LOCAL_AUTH_ID, LOCAL_EMAIL,
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const ACCESS_TOKEN: &str = "cursorAuth/accessToken";
const REFRESH_TOKEN: &str = "cursorAuth/refreshToken";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub path: PathBuf,
    pub originals: BTreeMap<String, Option<String>>,
    pub written: BTreeMap<String, String>,
}

fn read(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
}

fn wanted() -> BTreeMap<String, String> {
    BTreeMap::from([
        (ACCESS_TOKEN.to_owned(), local_token()),
        (REFRESH_TOKEN.to_owned(), local_token()),
        ("cursorAuth/cachedEmail".to_owned(), LOCAL_EMAIL.to_owned()),
        (
            "cursorAuth/cachedSignUpType".to_owned(),
            "Google".to_owned(),
        ),
        (
            "cursorAuth/stripeMembershipAuthId".to_owned(),
            LOCAL_AUTH_ID.to_owned(),
        ),
        (
            "cursorAuth/stripeMembershipType".to_owned(),
            "ultra".to_owned(),
        ),
        (
            "cursorAuth/stripeSubscriptionStatus".to_owned(),
            "active".to_owned(),
        ),
    ])
}

/// Only an unsigned-in client gets a local identity. Check and write in one transaction.
pub fn apply(path: &Path, held: Option<&Lease>) -> Result<Option<Lease>, String> {
    (|| -> rusqlite::Result<Option<Lease>> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")?;
        for key in [ACCESS_TOKEN, REFRESH_TOKEN] {
            if read(&transaction, key)?.is_some_and(|token| !token.trim().is_empty() && !is_local_token(&token)) {
                return Ok(None);
            }
        }
        let held = held.filter(|lease| lease.path == path);
        let stale = held.is_none() && read(&transaction, ACCESS_TOKEN)?.is_some_and(|token| is_local_token(&token));
        let written = wanted();
        let mut originals = held.map(|lease| lease.originals.clone()).unwrap_or_default();
        for (key, value) in &written {
            if !originals.contains_key(key) {
                originals.insert(key.clone(), if stale { None } else { read(&transaction, key)? });
            }
            transaction.execute("INSERT OR REPLACE INTO ItemTable(key, value) VALUES (?1, ?2)", [key, value])?;
        }
        transaction.commit()?;
        Ok(Some(Lease { path: path.to_owned(), originals, written }))
    })().map_err(|error| {
        log::warn!("Cursor local account could not be written: {error}");
        "cursor.errors.accountWrite".to_owned()
    })
}

pub fn release(lease: &Lease) -> Result<(), String> {
    if !lease.path.is_file() {
        return Ok(());
    }
    (|| -> rusqlite::Result<()> {
        let mut connection = Connection::open(&lease.path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A login made while injection was on belongs entirely to Cursor.
        for key in [ACCESS_TOKEN, REFRESH_TOKEN] {
            if read(&transaction, key)?
                .is_some_and(|token| !token.trim().is_empty() && !is_local_token(&token))
            {
                return Ok(());
            }
        }
        for (key, written) in &lease.written {
            if read(&transaction, key)?.as_ref() != Some(written) {
                continue;
            }
            match lease.originals.get(key).and_then(Option::as_ref) {
                Some(original) => {
                    transaction.execute(
                        "UPDATE ItemTable SET value = ?2 WHERE key = ?1",
                        [key, original],
                    )?;
                }
                None => {
                    transaction.execute("DELETE FROM ItemTable WHERE key = ?1", [key])?;
                }
            }
        }
        transaction.commit()
    })()
    .map_err(|error| {
        log::warn!("Cursor local account could not be restored: {error}");
        "cursor.errors.accountWrite".to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_identity_is_repaired_and_released_without_changing_user_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.vscdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable(key TEXT UNIQUE, value BLOB);
            INSERT INTO ItemTable VALUES ('cursorAuth/cachedEmail', 'previous@example.test');
            INSERT INTO ItemTable VALUES ('unrelated', 'keep');",
            )
            .unwrap();
        let lease = apply(&path, None).unwrap().unwrap();
        assert!(is_local_token(
            &read(&connection, ACCESS_TOKEN).unwrap().unwrap()
        ));
        connection
            .execute(
                "UPDATE ItemTable SET value = 'free' WHERE key = 'cursorAuth/stripeMembershipType'",
                [],
            )
            .unwrap();
        let lease = apply(&path, Some(&lease)).unwrap().unwrap();
        assert_eq!(
            read(&connection, "cursorAuth/stripeMembershipType")
                .unwrap()
                .as_deref(),
            Some("ultra")
        );
        release(&lease).unwrap();
        assert_eq!(read(&connection, ACCESS_TOKEN).unwrap(), None);
        assert_eq!(
            read(&connection, "cursorAuth/cachedEmail")
                .unwrap()
                .as_deref(),
            Some("previous@example.test")
        );
        assert_eq!(
            read(&connection, "unrelated").unwrap().as_deref(),
            Some("keep")
        );
    }

    #[test]
    fn official_access_or_refresh_tokens_are_never_overwritten() {
        for key in [ACCESS_TOKEN, REFRESH_TOKEN] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("state.vscdb");
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("CREATE TABLE ItemTable(key TEXT UNIQUE, value BLOB)")
                .unwrap();
            connection
                .execute("INSERT INTO ItemTable VALUES (?1, 'official-token')", [key])
                .unwrap();
            assert!(apply(&path, None).unwrap().is_none());
            assert_eq!(
                read(&connection, key).unwrap().as_deref(),
                Some("official-token")
            );
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM ItemTable", [], |row| row
                        .get::<_, i32>(0))
                    .unwrap(),
                1
            );
        }
    }

    #[test]
    fn releasing_keeps_a_new_official_login_and_user_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.vscdb");
        let lease = apply(&path, None).unwrap().unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "UPDATE ItemTable SET value = 'edited' WHERE key = 'cursorAuth/cachedEmail'",
                [],
            )
            .unwrap();
        release(&lease).unwrap();
        assert_eq!(
            read(&connection, "cursorAuth/cachedEmail")
                .unwrap()
                .as_deref(),
            Some("edited")
        );
        let lease = apply(&path, None).unwrap().unwrap();
        connection
            .execute(
                "UPDATE ItemTable SET value = 'new-official-token' WHERE key = ?1",
                [ACCESS_TOKEN],
            )
            .unwrap();
        release(&lease).unwrap();
        assert_eq!(
            read(&connection, ACCESS_TOKEN).unwrap().as_deref(),
            Some("new-official-token")
        );
        assert_eq!(
            read(&connection, "cursorAuth/stripeMembershipType")
                .unwrap()
                .as_deref(),
            Some("ultra")
        );
    }

    #[test]
    fn lost_leases_do_not_preserve_a_stale_local_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.vscdb");
        apply(&path, None).unwrap();
        let recovered = apply(&path, None).unwrap().unwrap();
        release(&recovered).unwrap();
        let connection = Connection::open(&path).unwrap();
        assert_eq!(read(&connection, ACCESS_TOKEN).unwrap(), None);
    }
}
