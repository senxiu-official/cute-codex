use anyhow::Result;
use pretty_assertions::assert_eq;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn sole_thread_retention_probe_has_no_home_side_effects() -> Result<()> {
    let home = TempDir::new()?;
    std::fs::write(home.path().join("config.toml"), "invalid TOML [")?;
    std::fs::write(home.path().join("auth.json"), "invalid JSON")?;
    std::fs::write(home.path().join(".env"), "invalid dotenv syntax '")?;
    let before = std::fs::read_dir(home.path())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
    let output = Command::new(codex_utils_cargo_bin::cargo_bin("codex-app-server")?)
        .env("CODEX_HOME", home.path())
        .arg("--thread-retention-capability")
        .output()?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{\"threadRetentionVersion\":1}\n");
    assert!(output.stderr.is_empty());
    let after = std::fs::read_dir(home.path())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
    assert_eq!(after, before);
    Ok(())
}

#[test]
fn thread_retention_capability_does_not_load_config_or_auth() -> Result<()> {
    let home = TempDir::new()?;
    std::fs::write(home.path().join("config.toml"), "invalid TOML [")?;
    std::fs::write(home.path().join("auth.json"), "invalid JSON")?;
    let output = Command::new(codex_utils_cargo_bin::cargo_bin("codex-app-server")?)
        .env("CODEX_HOME", home.path())
        .args([
            "--thread-retention-capability",
            "--auth-file",
            "invalid-relative-auth-path",
        ])
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout)?,
        serde_json::json!({"threadRetentionVersion": 1})
    );
    Ok(())
}
