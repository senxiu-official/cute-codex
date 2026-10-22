//! Cutex fork: API-key fallback from the launch-selected `--auth-file`.
//!
//! An env-key provider (`requires_openai_auth = false`, `env_key` set) normally reads its key from
//! the process environment. When that variable is missing or empty, the key stored as
//! `OPENAI_API_KEY` in the explicit `--auth-file` is used instead, so a supervisor can stop passing
//! provider keys through the environment. `codex-login` installs the reader when it fixes the
//! auth-file selection; without `--auth-file` nothing changes.

use std::sync::OnceLock;

/// Reads the selected auth file's `OPENAI_API_KEY`, if any.
pub type AuthFileApiKeyReader = fn() -> Option<String>;

static READER: OnceLock<AuthFileApiKeyReader> = OnceLock::new();

/// Installs the auth-file key reader once per process. Later calls are ignored.
pub fn set_auth_file_api_key_reader(reader: AuthFileApiKeyReader) {
    let _ = READER.set(reader);
}

pub(crate) fn auth_file_api_key() -> Option<String> {
    READER
        .get()
        .and_then(|reader| reader())
        .filter(|key| !key.trim().is_empty())
}
