// Aggregates all former standalone integration tests as modules.
mod auth_refresh;
mod device_code_login;
mod login_proxy_fallback;
mod login_server_e2e;
mod logout;
#[cfg(target_os = "linux")]
mod selected_file;
