//! Cutex fork: thread_history migration numbering guards.
//!
//! The fork owns thread_history versions 7 to 9; upstream's `0008_thread_turn_root` is renumbered
//! to 10. Live databases record these versions with their checksums, so the fork files must stay
//! byte-identical and every version unique.

use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use sqlx::migrate::Migrator;
use std::borrow::Cow;

use super::THREAD_HISTORY_MIGRATOR;

/// SHA-384 of the shipped migration files (sqlx checksum), as recorded by native r47 to r52.
const FORK_THREAD_HISTORY_CHECKSUMS: [(i64, &str, &str); 4] = [
    (
        7,
        "external input projection",
        "5a6238de70773eb42da1e39e061aced07c4d8220f9d85ce6f9121e910a437bbe97c6996c0ed1a42aa224fba0e64bee7e",
    ),
    (
        8,
        "presentations",
        "56137b0b23313d1b6b9258fdd5c9ec86ad9ca88554283eb33699b1c207f3ee73d9418864408e180270b6c5ae9957a255",
    ),
    (
        9,
        "thread item lifecycle timestamps",
        "9334adb4cda792116a47b69a19e3840ae3806b08773ef1794043fe0aa72891360acb1a0a8431084d53c568482c5fdd20",
    ),
    (
        10,
        "thread turn root",
        "94635cd58535adb1679f4ad07a9d14231b69004405fa88a22db1b6a7e5feac8b6b66ad97ea16fce464b99bff5b3548d3",
    ),
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn thread_history_migrator_through(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            THREAD_HISTORY_MIGRATOR
                .migrations
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: THREAD_HISTORY_MIGRATOR.locking,
        table_name: THREAD_HISTORY_MIGRATOR.table_name.clone(),
        create_schemas: THREAD_HISTORY_MIGRATOR.create_schemas.clone(),
        no_tx: THREAD_HISTORY_MIGRATOR.no_tx,
    }
}

#[test]
fn thread_history_versions_are_unique_and_fork_checksums_are_pinned() {
    let versions = THREAD_HISTORY_MIGRATOR
        .migrations
        .iter()
        .map(|migration| migration.version)
        .collect::<Vec<_>>();
    assert_eq!(versions, (1..=10).collect::<Vec<_>>());
    for (version, description, checksum) in FORK_THREAD_HISTORY_CHECKSUMS {
        let migration = THREAD_HISTORY_MIGRATOR
            .migrations
            .iter()
            .find(|migration| migration.version == version)
            .expect("pinned migration should exist");
        assert_eq!(
            (migration.description.as_ref(), hex(&migration.checksum)),
            (description, checksum.to_string()),
            "thread_history migration {version}"
        );
    }
}

#[tokio::test]
async fn r51_thread_history_db_migrates_to_turn_roots_and_stays_writable_by_r51() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.as_path().abs());

    // A native r47 to r52 database: fork migrations 7 to 9 applied.
    let r51_migrator = thread_history_migrator_through(/*version*/ 9);
    let pool = sqlite
        .open_thread_history_db(&r51_migrator, /*telemetry_override*/ None)
        .await
        .expect("r51 thread history migrations should apply");
    sqlx::query("INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status) VALUES ('thread-1', 'turn-1', 0, 'completed')")
        .execute(&pool)
        .await
        .expect("r51 writer inserts a turn");
    pool.close().await;

    // r53 opens it: only version 10 is new.
    let pool = sqlite
        .open_thread_history_db(&THREAD_HISTORY_MIGRATOR, /*telemetry_override*/ None)
        .await
        .expect("r53 should migrate an r51 thread history database");
    let applied =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("applied versions should load");
    assert_eq!(applied, (1..=10).collect::<Vec<_>>());
    sqlx::query("INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status, root_turn_id) VALUES ('thread-1', 'turn-2', 1, 'completed', 'turn-1')")
        .execute(&pool)
        .await
        .expect("r53 writer records a root turn");
    pool.close().await;

    // Rollback: r51 / r52 reopen the database (ignore_missing) and keep writing turns.
    let pool = sqlite
        .open_thread_history_db(&r51_migrator, /*telemetry_override*/ None)
        .await
        .expect("r51 should open a database migrated by r53");
    sqlx::query("INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status) VALUES ('thread-1', 'turn-3', 2, 'completed')")
        .execute(&pool)
        .await
        .expect("r51 writer still inserts turns without roots");
    let roots = sqlx::query_scalar::<_, Option<String>>(
        "SELECT root_turn_id FROM thread_turns ORDER BY rollout_ordinal",
    )
    .fetch_all(&pool)
    .await
    .expect("roots should load");
    assert_eq!(roots, vec![None, Some("turn-1".to_string()), None]);
    pool.close().await;
}
