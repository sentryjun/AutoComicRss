// SPDX-License-Identifier: GPL-2.0-only
use autocomic_rss::{domain::SubscriptionInput, storage::Store};
use std::process::Command;

#[test]
fn migration_cli_requires_explicit_offline_mode_and_environment() {
    let executable = env!("CARGO_BIN_EXE_autocomic-rss");
    let output = Command::new(executable).args(["migrate"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--offline"));
    let output = Command::new(executable)
        .args(["migrate", "--offline"])
        .env_remove("MIGRATION_SOURCE_URL")
        .env_remove("MIGRATION_TARGET_URL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("MIGRATION_SOURCE_URL is required"));
}

#[tokio::test]
#[ignore = "requires a dedicated MySQL 8.4 server; see tests/README.md"]
async fn migration_cli_copies_and_rejects_populated_target() {
    let base = std::env::var("AUTOCOMIC_TEST_MYSQL_URL").expect("dedicated test server required");
    let admin = sqlx::MySqlPool::connect(&base).await.unwrap();
    let name = format!("autocomic_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&base).unwrap();
    url.set_path(&format!("/{name}"));
    let directory = std::env::temp_dir().join(format!("migration-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let source_url = format!("sqlite://{}", directory.join("source.db").display());
    let source = Store::open(&source_url).await.unwrap();
    let sub = source
        .create(SubscriptionInput {
            title: "迁移验证".into(),
            rss_url: "https://example.invalid/rss".into(),
            season: 1,
            episode_offset: 0,
            include: vec![],
            exclude: vec![],
            enabled: false,
        })
        .await
        .unwrap();
    source.pool.close().await;
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_autocomic-rss"))
            .args(["migrate", "--offline"])
            .env("MIGRATION_SOURCE_URL", &source_url)
            .env("MIGRATION_TARGET_URL", url.as_str())
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("verified and committed"));
    let target = Store::open(url.as_str()).await.unwrap();
    assert_eq!(
        target.subscription(&sub.id).await.unwrap().input.title,
        "迁移验证"
    );
    let result = run();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must be empty"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains(url.as_str()));
    assert_eq!(target.subscriptions().await.unwrap().len(), 1);
    target.pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    std::fs::remove_dir_all(directory).unwrap();
}
