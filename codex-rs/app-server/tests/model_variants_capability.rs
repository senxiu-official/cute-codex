use anyhow::Result;
use pretty_assertions::assert_eq;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn model_variants_capability_does_not_load_config_or_auth() -> Result<()> {
    let home = TempDir::new()?;
    std::fs::write(home.path().join("config.toml"), "invalid TOML [")?;
    std::fs::write(home.path().join("auth.json"), "invalid JSON")?;
    let output = Command::new(codex_utils_cargo_bin::cargo_bin("codex-app-server")?)
        .env("CODEX_HOME", home.path())
        .args([
            "--model-variants-capability",
            "--auth-file",
            "invalid-relative-auth-path",
        ])
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout)?,
        serde_json::json!({"modelVariants": true})
    );
    Ok(())
}
