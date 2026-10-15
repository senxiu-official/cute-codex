#![expect(clippy::unwrap_used, reason = "private binding file fixture setup")]
use super::*;
use std::os::unix::fs::PermissionsExt;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("binding.json");
    std::fs::write(
        &path,
        r#"{"version":1,"ownerId":"owner","threadId":"thread","runtimeGeneration":7}"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    (dir, path)
}

#[test]
fn binding_is_a_validated_launch_snapshot() {
    let (_dir, path) = fixture();
    let binding = ExternalInputBinding::load(&path).unwrap();
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(binding.owner_id, "owner");
    assert_eq!(binding.runtime_generation, 7);
    assert!(ExternalInputBinding::load(&path).is_err());
}

#[test]
fn rejects_nonprivate_directory_file_and_symlinks() {
    let (dir, path) = fixture();
    for mode in [0o644, 0o660, 0o400, 0o1600] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(ExternalInputBinding::load(&path).is_err());
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ExternalInputBinding::load(&path).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(ExternalInputBinding::load(&alias).is_err());
    let parent_alias = dir.path().join("parent-alias");
    std::os::unix::fs::symlink(dir.path(), &parent_alias).unwrap();
    assert!(ExternalInputBinding::load(&parent_alias.join("binding.json")).is_err());
}

#[test]
fn rejects_unknown_fields_version_and_unbounded_launch_data() {
    let (_dir, path) = fixture();
    for text in [
        r#"{"version":2,"ownerId":"owner","threadId":"thread","runtimeGeneration":7}"#.to_string(),
        r#"{"version":1,"ownerId":"owner","threadId":"thread","runtimeGeneration":7,"role":"system"}"#.to_string(),
        " ".repeat(4097),
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(ExternalInputBinding::load(&path).is_err());
    }
}

#[test]
fn receiver_policy_distinguishes_default_positive_and_off_and_rejects_invalid() {
    use codex_core::context::CanonicalBytePolicy;
    let (_dir, path) = fixture();
    assert_eq!(
        ExternalInputBinding::load(&path)
            .unwrap()
            .canonical_byte_limit,
        CanonicalBytePolicy::default()
    );
    let mut binding: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for value in [
        serde_json::json!(1),
        serde_json::json!(10000),
        serde_json::json!(100000),
        serde_json::json!(4294967295_u64),
        serde_json::json!("off"),
    ] {
        binding["canonicalByteLimit"] = value.clone();
        std::fs::write(&path, serde_json::to_vec(&binding).unwrap()).unwrap();
        let loaded = ExternalInputBinding::load(&path).unwrap();
        assert_eq!(
            loaded.canonical_byte_limit,
            serde_json::from_value::<CanonicalBytePolicy>(value).unwrap()
        );
    }
    for value in [
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!(4294967296_u64),
        serde_json::json!(null),
        serde_json::json!(false),
        serde_json::json!("10000"),
        serde_json::json!("OFF"),
        serde_json::json!({}),
    ] {
        binding["canonicalByteLimit"] = value;
        std::fs::write(&path, serde_json::to_vec(&binding).unwrap()).unwrap();
        assert!(ExternalInputBinding::load(&path).is_err());
    }
}
