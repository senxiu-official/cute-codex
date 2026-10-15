use super::linux::SelectedFile;
use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;

fn auth(label: &str) -> AuthDotJson {
    serde_json::from_value(serde_json::json!({"OPENAI_API_KEY":label})).unwrap()
}

#[test]
fn selected_file_atomic_save_reload_and_delete_preserve_other_account() {
    let home = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let first = SelectedFile::open(home.path().join("first.json")).unwrap();
    let second = SelectedFile::open(home.path().join("second.json")).unwrap();
    let sentinel = home.path().join("auth.json");
    fs::write(&sentinel, b"default sentinel").unwrap();
    first.save(&auth("dummy-first")).unwrap();
    second.save(&auth("dummy-second")).unwrap();
    let old_inode = fs::metadata(home.path().join("first.json")).unwrap().ino();
    first.save(&auth("dummy-refreshed")).unwrap();
    assert_ne!(
        old_inode,
        fs::metadata(home.path().join("first.json")).unwrap().ino()
    );
    assert_eq!(first.load().unwrap(), Some(auth("dummy-refreshed")));
    assert_eq!(second.load().unwrap(), Some(auth("dummy-second")));
    assert!(first.delete().unwrap());
    assert_eq!(first.load().unwrap(), None);
    assert_eq!(second.load().unwrap(), Some(auth("dummy-second")));
    assert_eq!(fs::read(sentinel).unwrap(), b"default sentinel");
}

#[test]
fn selected_file_rejects_symlink_permissions_and_hardlinks_without_mutation() {
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let private = root.path().join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let target = private.join("auth.json");
    let storage = SelectedFile::open(target.clone()).unwrap();
    storage.save(&auth("dummy")).unwrap();
    let before = fs::read(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(storage.load().is_err());
    assert!(storage.save(&auth("wrong")).is_err());
    assert!(storage.delete().is_err());
    assert_eq!(fs::read(&target).unwrap(), before);
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&target, private.join("link")).unwrap();
    assert!(storage.load().is_err());
    fs::remove_file(private.join("link")).unwrap();
    let other = private.join("other");
    fs::rename(&target, &other).unwrap();
    symlink(&other, &target).unwrap();
    assert!(storage.load().is_err());
    assert!(storage.save(&auth("wrong")).is_err());
    assert!(storage.delete().is_err());
    assert_eq!(fs::read(&other).unwrap(), before);
    symlink(&private, root.path().join("alias")).unwrap();
    assert!(SelectedFile::open(root.path().join("alias/new.json")).is_err());
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SelectedFile::open(private.join("missing.json")).is_err());
}

#[test]
fn selected_file_pins_parent_and_accepts_safe_refresh_replacement() {
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = root.path().join("private");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let storage = SelectedFile::open(path.join("auth.json")).unwrap();
    storage.save(&auth("first")).unwrap();
    let concurrent = SelectedFile::open(path.join("auth.json")).unwrap();
    concurrent.save(&auth("new-token-same-custody")).unwrap();
    assert_eq!(
        storage.load().unwrap(),
        Some(auth("new-token-same-custody"))
    );
    fs::rename(&path, root.path().join("old")).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    for result in [
        storage.load().map(|_| ()),
        storage.save(&auth("wrong")),
        storage.delete().map(|_| ()),
    ] {
        assert!(result.is_err());
    }
    assert!(!path.join("auth.json").exists());
    assert!(root.path().join("old/auth.json").exists());
}

#[test]
fn selected_file_launch_is_once_and_never_falls_back_to_other_storage() {
    // Keep the immutable launch selection out of a shared libtest process too.
    if std::env::var_os("SELECTED_FILE_UNIT_CHILD").is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "auth::selected_file::tests::selected_file_launch_is_once_and_never_falls_back_to_other_storage"])
            .env_clear()
            .env("HOME", std::env::temp_dir())
            .env("TMPDIR", std::env::temp_dir())
            .env("SELECTED_FILE_UNIT_CHILD", "1")
            .output().unwrap();
        assert!(
            result.status.success(),
            "isolated launch-selection child failed"
        );
        return;
    }
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    fs::write(root.path().join("auth.json"), b"default sentinel").unwrap();
    configure_auth_file(Some(root.path().join("selected.json"))).unwrap();
    assert!(configure_auth_file(None).is_err());
    let file = super::super::storage::create_auth_storage(
        root.path().to_owned(),
        AuthCredentialsStoreMode::File,
        Default::default(),
    );
    assert_eq!(file.load().unwrap(), None);
    for mode in [
        AuthCredentialsStoreMode::Auto,
        AuthCredentialsStoreMode::Keyring,
        AuthCredentialsStoreMode::Ephemeral,
    ] {
        assert!(validate_mode(mode).is_err());
        let storage = super::super::storage::create_auth_storage(
            root.path().to_owned(),
            mode,
            Default::default(),
        );
        assert!(storage.load().is_err());
        assert!(storage.save(&auth("wrong")).is_err());
        assert!(storage.delete().is_err());
    }
    assert_eq!(
        fs::read(root.path().join("auth.json")).unwrap(),
        b"default sentinel"
    );
}
