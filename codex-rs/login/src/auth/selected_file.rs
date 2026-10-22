//! Immutable, launch-only file custody. This is not a repository/RPC setting.
use super::storage::AuthDotJson;
use super::storage::AuthStorageBackend;
use codex_config::types::AuthCredentialsStoreMode;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

static SELECTION: OnceLock<Option<Arc<dyn AuthStorageBackend>>> = OnceLock::new();

/// Selects file custody once, before any configuration or authentication is loaded.
/// Passing no path preserves the default credential-store behavior.
pub fn configure_auth_file(path: Option<PathBuf>) -> io::Result<()> {
    let storage = path.map(open_selected).transpose()?;
    let explicit = storage.is_some();
    SELECTION
        .set(storage)
        .map_err(|_| io::Error::other("auth-file selection is already fixed"))?;
    if explicit {
        // Env-key providers may take their key from this file instead of the environment.
        codex_model_provider_info::set_auth_file_api_key_reader(selected_file_api_key);
    }
    Ok(())
}

/// The selected file's `OPENAI_API_KEY`. Read on demand so a rewritten file is honored.
fn selected_file_api_key() -> Option<String> {
    let storage = selected()?;
    match storage.load() {
        Ok(auth) => auth.and_then(|auth| auth.openai_api_key),
        Err(error) => {
            tracing::warn!(%error, "failed to read the API key from the selected auth file");
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn open_selected(path: PathBuf) -> io::Result<Arc<dyn AuthStorageBackend>> {
    Ok(Arc::new(linux::SelectedFile::open(path)?))
}

#[cfg(windows)]
fn open_selected(path: PathBuf) -> io::Result<Arc<dyn AuthStorageBackend>> {
    Ok(Arc::new(windows::SelectedFile::open(path)?))
}

#[cfg(not(any(target_os = "linux", windows)))]
fn open_selected(_path: PathBuf) -> io::Result<Arc<dyn AuthStorageBackend>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "explicit auth-file custody is supported on Linux and Windows",
    ))
}

pub(super) fn selected() -> Option<Arc<dyn AuthStorageBackend>> {
    SELECTION.get_or_init(|| None).clone()
}

pub(super) fn validate_mode(mode: AuthCredentialsStoreMode) -> io::Result<()> {
    if selected().is_some() && mode != AuthCredentialsStoreMode::File {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "--auth-file requires file credential storage",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct ConflictingStorage;
impl AuthStorageBackend for ConflictingStorage {
    fn load(&self) -> io::Result<Option<AuthDotJson>> {
        Err(conflict())
    }
    fn save(&self, _: &AuthDotJson) -> io::Result<()> {
        Err(conflict())
    }
    fn delete(&self) -> io::Result<bool> {
        Err(conflict())
    }
}
fn conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "selected auth-file conflicts with another credential store",
    )
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, target_os = "linux"))]
#[path = "selected_file_tests.rs"]
mod tests;
#[cfg(windows)]
mod windows;

/// Whether local launch owns an explicit auth file; remote/daemon reuse cannot honor it.
pub fn explicit_auth_file_selected() -> bool {
    selected().is_some()
}
