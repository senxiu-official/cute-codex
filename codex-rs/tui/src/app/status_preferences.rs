//! Optional Cutex destination for presentation preferences only.
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::legacy_core::config::edit::ConfigEditsBuilder;

fn destination(
    shared_path: Option<OsString>,
    home: Option<PathBuf>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = shared_path {
        let path = PathBuf::from(path);
        anyhow::ensure!(
            path.is_absolute(),
            "CUTEX_STATUS_LINE_CONFIG must be absolute"
        );
        return Ok(Some(path));
    }
    Ok(home.map(|home| home.join(".cutex/status-line.toml")))
}

fn default_home() -> Option<PathBuf> {
    // Unit tests must not read or overwrite the developer's real preferences.
    if cfg!(test) { None } else { dirs::home_dir() }
}

pub(crate) fn reload(tui: &mut codex_config::types::Tui) -> anyhow::Result<()> {
    if let Some(path) = destination(std::env::var_os("CUTEX_STATUS_LINE_CONFIG"), default_home())? {
        reload_from(tui, &path)?;
    }
    Ok(())
}

fn reload_from(tui: &mut codex_config::types::Tui, path: &Path) -> anyhow::Result<()> {
    #[derive(serde::Deserialize, Default)]
    struct Preferences {
        status_line: Option<Vec<String>>,
        status_line_use_colors: Option<bool>,
    }
    #[derive(serde::Deserialize, Default)]
    struct File {
        #[serde(default)]
        tui: Preferences,
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let saved: File = toml::from_str(&text)?;
    if let Some(items) = saved.tui.status_line {
        tui.status_line = Some(items);
    }
    if let Some(colors) = saved.tui.status_line_use_colors {
        tui.status_line_use_colors = colors;
    }
    Ok(())
}

pub(super) async fn save(
    default_path: &Path,
    shared_path: Option<OsString>,
    ids: &[String],
    use_theme_colors: bool,
) -> anyhow::Result<()> {
    let path =
        destination(shared_path, default_home())?.unwrap_or_else(|| default_path.to_path_buf());
    ConfigEditsBuilder::for_config_path(&path)
        .with_edits([
            crate::legacy_core::config::edit::status_line_items_edit(ids),
            crate::legacy_core::config::edit::status_line_use_colors_edit(use_theme_colors),
        ])
        .apply()
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "status_preferences_tests.rs"]
mod tests;
