use std::{collections::BTreeSet, path::Path};

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{
    PgPool, SqlStr,
    migrate::{Migration, MigrationType, Migrator},
};

use super::{TEST_MIGRATOR, TestDatabase};

pub(super) const USER_DELETION_VERSION: i64 = 202609130002;
const UPSTREAM_VERSION_LIMIT: i64 = 10000;

#[test]
fn migration_names_use_unique_upstream_or_fork_versions() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut versions = BTreeSet::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "sql") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (number, description) = name.split_once('_').expect("version_description.sql");
        assert!(number.bytes().all(|byte| byte.is_ascii_digit()), "{name}");
        assert!(
            description.split('_').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            }),
            "migration description must use lowercase snake_case: {name}"
        );
        match number.len() {
            4 => assert!(number != "0000", "{name}"),
            12 => {
                NaiveDate::parse_from_str(&number[..8], "%Y%m%d")
                    .expect("fork migration date must be valid YYYYMMDD");
                assert!(&number[8..] != "0000", "{name}");
            }
            _ => panic!("expected upstream NNNN or fork YYYYMMDDNNNN: {name}"),
        }
        let version = number.parse::<i64>().unwrap();
        assert!(
            versions.insert(version),
            "duplicate migration version: {version}"
        );
    }
    assert_eq!(
        versions.into_iter().collect::<Vec<_>>(),
        TEST_MIGRATOR
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn late_upstream_migrations_run_once_and_match_a_fresh_install() {
    let Some(upgraded) = TestDatabase::create("late_upstream").await else {
        return;
    };
    let before = applied_migrations(&upgraded.pool).await;
    assert!(
        before
            .iter()
            .any(|(version, _, _)| *version >= UPSTREAM_VERSION_LIMIT)
    );

    // 只在测试中追加上游样例，分别覆盖独立表和与 fork 共同扩展的表。
    let next_upstream = TEST_MIGRATOR
        .iter()
        .filter(|migration| migration.version < UPSTREAM_VERSION_LIMIT)
        .map(|migration| migration.version)
        .max()
        .unwrap()
        + 1;
    assert!(next_upstream + 1 < UPSTREAM_VERSION_LIMIT);
    let mut migrations = TEST_MIGRATOR.iter().cloned().collect::<Vec<_>>();
    migrations.extend([
        Migration::new(
            next_upstream,
            "upstream fixture table".into(),
            MigrationType::Simple,
            SqlStr::from_static(
                "create table upstream_migration_probe (id integer primary key);
             insert into upstream_migration_probe values (1);
             alter table admin_users add column upstream_note text not null default '';",
            ),
            false,
        ),
        Migration::new(
            next_upstream + 1,
            "upstream fixture data".into(),
            MigrationType::Simple,
            SqlStr::from_static(
                "insert into upstream_migration_probe values (2);
             alter table admin_users add constraint upstream_note_length_ck
               check (length(upstream_note) < 100);",
            ),
            false,
        ),
    ]);
    migrations.sort();
    let migrator = Migrator::with_migrations(migrations);
    migrator.run(&upgraded.pool).await.unwrap();
    let after = applied_migrations(&upgraded.pool).await;
    assert_eq!(after.len(), before.len() + 2);
    for applied in &before {
        assert!(
            after.contains(applied),
            "previously applied migration was changed"
        );
    }
    migrator.run(&upgraded.pool).await.unwrap();
    assert_eq!(applied_migrations(&upgraded.pool).await, after);

    let fresh = TestDatabase::create_with_migrator("fresh_upstream", &migrator)
        .await
        .unwrap();
    for pool in [&upgraded.pool, &fresh.pool] {
        let rows =
            sqlx::query_scalar::<_, i32>("select id from upstream_migration_probe order by id")
                .fetch_all(pool)
                .await
                .unwrap();
        assert_eq!(rows, [1, 2]);
        let owner: (String, String, String) = sqlx::query_as(
            "select role, password_hash, upstream_note from admin_users where id = 'test-owner'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            owner,
            ("admin".into(), "test-only-hash".into(), String::new())
        );
    }
    assert_eq!(
        schema_definition(&upgraded.pool).await,
        schema_definition(&fresh.pool).await
    );
    let fresh_versions = applied_migrations(&fresh.pool)
        .await
        .into_iter()
        .map(|(version, checksum, _)| (version, checksum))
        .collect::<Vec<_>>();
    let upgraded_versions = after
        .into_iter()
        .map(|(version, checksum, _)| (version, checksum))
        .collect::<Vec<_>>();
    assert_eq!(upgraded_versions, fresh_versions);
    upgraded.close().await;
    fresh.close().await;
}

async fn applied_migrations(pool: &PgPool) -> Vec<(i64, Vec<u8>, DateTime<Utc>)> {
    sqlx::query_as(
        "select version, checksum, installed_on from _sqlx_migrations where success order by version"
    ).fetch_all(pool).await.unwrap()
}

async fn schema_definition(pool: &PgPool) -> Vec<(String, String, String)> {
    // 列的物理顺序取决于升级路径；比较类型、默认值、约束和视图定义，不比较列序号。
    sqlx::query_as(
        "select table_name::text, column_name::text,
                concat(data_type, ':', udt_name, ':', is_nullable, ':', column_default,
                       ':', numeric_precision, ':', numeric_scale, ':', datetime_precision)
         from information_schema.columns where table_schema = current_schema()
         union all
         select c.relname::text, con.conname::text, pg_get_constraintdef(con.oid)
         from pg_constraint con join pg_class c on c.oid = con.conrelid
         join pg_namespace n on n.oid = c.relnamespace where n.nspname = current_schema()
         union all
         select table_name::text, 'view', view_definition
         from information_schema.views where table_schema = current_schema()
         order by 1, 2, 3",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}
