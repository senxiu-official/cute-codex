//! Optional Cutex release-version status.
//!
//! The launcher supplies the locally known-latest CLI version. A source or
//! standalone TUI without that input renders nothing rather than guessing that
//! it is current.

pub(crate) const LATEST_CLI_VERSION_ENV_VAR: &str = "CUTEX_LATEST_CLI_VERSION";

pub(crate) fn value(current_version: &str) -> Option<String> {
    let latest = std::env::var(LATEST_CLI_VERSION_ENV_VAR)
        .ok()
        .and_then(|value| {
            let value = value.trim().to_string();
            (value.len() <= 64).then_some(value)
        })
        .filter(|value| !value.is_empty())?;
    outdated_label(current_version, &latest)
}

pub(crate) fn append_automatic_item(
    items: &mut Vec<crate::bottom_pane::StatusLineItem>,
    status_line_disabled: bool,
) {
    let outdated = value(crate::version::CODEX_CLI_VERSION);
    append_automatic_item_with_label(items, status_line_disabled, outdated.as_deref())
}

pub(crate) fn append_automatic_item_with_label(
    items: &mut Vec<crate::bottom_pane::StatusLineItem>,
    status_line_disabled: bool,
    outdated_label: Option<&str>,
) {
    if !status_line_disabled
        && !items.contains(&crate::bottom_pane::StatusLineItem::CutexVersion)
        && outdated_label.is_some()
    {
        items.push(crate::bottom_pane::StatusLineItem::CutexVersion);
    }
}

pub(crate) fn outdated_label(current_version: &str, latest_version: &str) -> Option<String> {
    if current_version == latest_version {
        return None;
    }
    fn upstream(version: &str) -> &str {
        version
            .split_once('-')
            .map_or(version, |(upstream, _)| upstream)
    }
    let new_version = if upstream(current_version) == upstream(latest_version) {
        latest_version
            .split_once('-')
            .map_or(latest_version, |(_, release)| release)
    } else {
        latest_version
    };
    Some(format!("{current_version} (N: {new_version})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_release_is_hidden() {
        assert_eq!(outdated_label("0.156.1-r36", "0.156.1-r36"), None);
    }

    #[test]
    fn same_upstream_uses_compact_release_label() {
        assert_eq!(
            outdated_label("0.156.1-r35", "0.156.1-r36").unwrap(),
            "0.156.1-r35 (N: r36)"
        );
    }

    #[test]
    fn upstream_change_keeps_full_new_version() {
        assert_eq!(
            outdated_label("0.156.1-r35", "0.156.2-r1").unwrap(),
            "0.156.1-r35 (N: 0.156.2-r1)"
        );
    }

    #[test]
    fn automatic_item_respects_disabled_and_existing_selection() {
        use crate::bottom_pane::StatusLineItem;

        let mut items = Vec::new();
        append_automatic_item_with_label(
            &mut items,
            /*status_line_disabled*/ true,
            Some("0.0.0 (N: r1)"),
        );
        assert!(items.is_empty());

        append_automatic_item_with_label(
            &mut items,
            /*status_line_disabled*/ false,
            Some("0.0.0 (N: r1)"),
        );
        assert_eq!(items, vec![StatusLineItem::CutexVersion]);

        append_automatic_item_with_label(
            &mut items,
            /*status_line_disabled*/ false,
            Some("0.0.0 (N: r2)"),
        );
        assert_eq!(items, vec![StatusLineItem::CutexVersion]);
    }
}
