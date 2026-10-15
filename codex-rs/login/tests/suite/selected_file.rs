//! Process-isolated synthetic custody and native-managed refresh oracle.
use base64::Engine;
use codex_login::AuthCredentialsStoreMode;
use codex_login::AuthDotJson;
use codex_login::AuthKeyringBackendKind;
use codex_login::AuthManager;
use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn payload(account: &str) -> AuthDotJson {
    let claims = serde_json::json!({"sub":"synthetic-user", "https://api.openai.com/auth":{"chatgpt_account_id":account}});
    let jwt = format!(
        "e30.{}.dummy",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap())
    );
    serde_json::from_value(serde_json::json!({"auth_mode":"chatgpt", "OPENAI_API_KEY":null,
        "tokens":{"id_token":jwt,"access_token":"dummy-initial","refresh_token":"dummy-refresh","account_id":account},
        "last_refresh":chrono::Utc::now()})).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_file_native_refresh_two_accounts_one_home() {
    if let Ok(account) = std::env::var("SELECTED_FILE_TEST_ACCOUNT") {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        let selected = home.join(format!("{account}.json"));
        codex_login::configure_auth_file(Some(selected.clone())).unwrap();
        let manager = AuthManager::shared(
            home.clone(),
            true,
            AuthCredentialsStoreMode::File,
            None,
            None,
            AuthKeyringBackendKind::default(),
            codex_login::test_support::transport_default_auth_route_config(),
        )
        .await;
        assert_eq!(
            manager.auth_cached().unwrap().get_account_id(),
            Some(account.clone())
        );
        manager.refresh_token().await.unwrap();
        let refreshed = manager.auth_cached().unwrap().get_token_data().unwrap();
        assert_eq!(refreshed.access_token, "dummy-refreshed");
        assert_eq!(refreshed.refresh_token, "dummy-rotated");
        let disk: AuthDotJson = serde_json::from_slice(&fs::read(&selected).unwrap()).unwrap();
        assert_eq!(disk.tokens.unwrap(), refreshed);
        assert!(!manager.reload().await);
        // A different account is not accepted by guarded unauthorized recovery.
        codex_login::save_auth(
            &home,
            &payload("different-account"),
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        )
        .unwrap();
        assert!(manager.refresh_token().await.is_err());
        assert_eq!(
            manager.auth_cached().unwrap().get_account_id(),
            Some(account)
        );
        manager.logout().await.unwrap();
        assert!(!selected.exists());
        assert!(manager.auth_cached().is_none());
        assert_eq!(
            fs::read(home.join("auth.json")).unwrap(),
            b"default sentinel"
        );
        return;
    }
    let home = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    fs::write(home.path().join("auth.json"), b"default sentinel").unwrap();
    for account in ["account-a", "account-b"] {
        let selected = home.path().join(format!("{account}.json"));
        fs::write(&selected, serde_json::to_vec(&payload(account)).unwrap()).unwrap();
        fs::set_permissions(&selected, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let second_before = fs::read(home.path().join("account-b.json")).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"access_token":"dummy-refreshed","refresh_token":"dummy-rotated"}),
        ))
        .expect(2)
        .mount(&server)
        .await;
    for account in ["account-a", "account-b"] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "suite::selected_file::selected_file_native_refresh_two_accounts_one_home",
            ])
            .env_clear()
            .env("HOME", home.path())
            .env("TMPDIR", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("SELECTED_FILE_TEST_ACCOUNT", account)
            .env("CODEX_API_KEY", "must-not-override-selected-file")
            .env(
                codex_login::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR,
                format!("{}/oauth/token", server.uri()),
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "synthetic child failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if account == "account-a" {
            assert_eq!(
                fs::read(home.path().join("account-b.json")).unwrap(),
                second_before
            );
        }
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "dummy-refresh");
    }
    assert_eq!(
        fs::read(home.path().join("auth.json")).unwrap(),
        b"default sentinel"
    );
    assert!(!Path::new(home.path()).join("account-a.json").exists());
    assert!(!Path::new(home.path()).join("account-b.json").exists());
}
