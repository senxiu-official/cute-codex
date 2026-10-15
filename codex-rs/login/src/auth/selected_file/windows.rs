//! Select the native Windows file credential store independently of CODEX_HOME.
//! Keep the chosen directory open without delete sharing for the owner's lifetime.
use super::*;
use crate::auth::storage::FileAuthStorage;
use std::fs::File;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;

#[derive(Debug)]
pub(super) struct SelectedFile {
    storage: FileAuthStorage,
    _directory: File,
}

impl SelectedFile {
    pub(super) fn open(path: PathBuf) -> io::Result<Self> {
        if !path.is_absolute() || path.file_name().is_none_or(|name| name != "auth.json") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows --auth-file requires an absolute auth.json path",
            ));
        }
        let parent = path
            .parent()
            .ok_or(io::ErrorKind::InvalidInput)?
            .canonicalize()?;
        // FILE_FLAG_BACKUP_SEMANTICS opens directories. Read access, rather
        // than metadata-only access, enforces the omitted FILE_SHARE_DELETE.
        let directory = OpenOptions::new()
            .read(true)
            .share_mode(/*share_mode*/ 3)
            .custom_flags(/*flags*/ 0x0200_0000)
            .open(&parent)?;
        Ok(Self {
            storage: FileAuthStorage::new(parent),
            _directory: directory,
        })
    }
}

impl AuthStorageBackend for SelectedFile {
    fn load(&self) -> io::Result<Option<AuthDotJson>> {
        self.storage.load()
    }

    fn save(&self, auth: &AuthDotJson) -> io::Result<()> {
        self.storage.save(auth)
    }

    fn delete(&self) -> io::Result<bool> {
        self.storage.delete()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_store_round_trip_and_delete_preserve_other_home() -> io::Result<()> {
        let selected = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let other_auth = other.path().join("auth.json");
        std::fs::write(&other_auth, b"unchanged")?;
        let store = SelectedFile::open(selected.path().join("auth.json"))?;
        assert!(store.load()?.is_none());
        let auth: AuthDotJson = serde_json::from_value(serde_json::json!({
            "OPENAI_API_KEY": "test-only-selected-key"
        }))?;
        store.save(&auth)?;
        assert_eq!(
            serde_json::to_value(store.load()?)?,
            serde_json::to_value(Some(auth))?
        );
        assert!(store.delete()?);
        assert!(store.load()?.is_none());
        assert_eq!(std::fs::read(other_auth)?, b"unchanged");
        Ok(())
    }

    #[test]
    fn selected_directory_cannot_be_replaced_while_in_use() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let parent = root.path().join("profile");
        std::fs::create_dir(&parent)?;
        let store = SelectedFile::open(parent.join("auth.json"))?;
        let renamed = root.path().join("replaced");
        assert!(std::fs::rename(&parent, &renamed).is_err());
        drop(store);
        std::fs::rename(parent, renamed)?;
        Ok(())
    }
}
