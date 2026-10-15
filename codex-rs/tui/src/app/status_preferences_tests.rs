use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn status_preferences_share_order_without_changing_runtime_config() {
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("config.toml");
    let shared = root.path().join("status-line.toml");
    std::fs::write(&local, "model='unchanged'\n").unwrap();
    let ids = vec!["current-dir".into(), "cutex_welcome".into()];
    save(
        &local,
        Some(shared.clone().into_os_string()),
        &ids,
        /*use_theme_colors*/ false,
    )
    .await
    .unwrap();
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(&shared).unwrap()).unwrap();
    assert_eq!(
        value["tui"]["status_line"]
            .clone()
            .try_into::<Vec<String>>()
            .unwrap(),
        ids
    );
    assert_eq!(
        value["tui"]["status_line_use_colors"].as_bool(),
        Some(false)
    );
    assert_eq!(
        std::fs::read_to_string(&local).unwrap(),
        "model='unchanged'\n"
    );
    save(
        &local,
        Some(shared.clone().into_os_string()),
        &[],
        /*use_theme_colors*/ true,
    )
    .await
    .unwrap();
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(&shared).unwrap()).unwrap();
    assert_eq!(value["tui"]["status_line"].as_array().unwrap().len(), 0);
    save(
        &local, /*shared_path*/ None, &ids, /*use_theme_colors*/ true,
    )
    .await
    .unwrap();
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(&local).unwrap()).unwrap();
    assert_eq!(value["model"].as_str(), Some("unchanged"));
    assert_eq!(
        value["tui"]["status_line"]
            .clone()
            .try_into::<Vec<String>>()
            .unwrap(),
        ids
    );
}

#[tokio::test]
async fn saved_preferences_override_other_homes_and_reload_without_resetting_theme() {
    let root = tempfile::tempdir().unwrap();
    let shared = root.path().join("status-line.toml");
    let first_home = root.path().join("first/config.toml");
    let ids = vec![
        "cutex_profile".into(),
        "current-dir".into(),
        "cutex_welcome".into(),
    ];
    save(
        &first_home,
        Some(shared.clone().into_os_string()),
        &ids,
        /*use_theme_colors*/ false,
    )
    .await
    .unwrap();
    let mut other = codex_config::types::Tui {
        status_line: Some(vec!["model-name".into()]),
        theme: Some("preserved-theme".into()),
        ..Default::default()
    };
    reload_from(&mut other, &shared).unwrap();
    assert_eq!(other.status_line, Some(ids));
    assert!(!other.status_line_use_colors);
    assert_eq!(other.theme, Some("preserved-theme".into()));
    assert!(!first_home.exists());
    save(
        &first_home,
        Some(shared.clone().into_os_string()),
        &[],
        /*use_theme_colors*/ true,
    )
    .await
    .unwrap();
    reload_from(&mut other, &shared).unwrap();
    assert_eq!(other.status_line, Some(vec![]));
    let before = other.clone();
    std::fs::write(&shared, "[broken").unwrap();
    assert!(reload_from(&mut other, &shared).is_err());
    assert_eq!(other, before);
}

#[test]
fn direct_entry_uses_shared_home_and_explicit_isolated_destination_wins() {
    let home = tempfile::tempdir().unwrap();
    let shared = home.path().join(".cutex/status-line.toml");
    assert_eq!(
        destination(None, Some(home.path().into())).unwrap(),
        Some(shared)
    );
    let isolated = home.path().join("isolated/status-line.toml");
    assert_eq!(
        destination(
            Some(isolated.clone().into_os_string()),
            Some(home.path().into())
        )
        .unwrap(),
        Some(isolated)
    );
    assert!(destination(Some("relative.toml".into()), Some(home.path().into())).is_err());
}

#[tokio::test]
async fn cutex_presentation_survives_statusline_reordering() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("status-line.toml");
    std::fs::write(&path,"[tui.presentation.cutex_weekly_limit]\nbar_width=10\n[tui.presentation.cutex_weekly_limit.style]\nfg='#F7B3CD'\n").unwrap();
    let before: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let ids = vec![
        "cutex_weekly_limit".into(),
        "cutex_newline".into(),
        "cutex_profile".into(),
    ];
    save(&path, Some(path.clone().into_os_string()), &ids, true)
        .await
        .unwrap();
    let after: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(after["tui"]["presentation"], before["tui"]["presentation"]);
    assert_eq!(
        after["tui"]["status_line"]
            .clone()
            .try_into::<Vec<String>>()
            .unwrap(),
        ids
    );
}
