//! Cutex fork: runtime-registered non-inheritable variables (the active provider's `env_key`).

use std::collections::HashMap;
use std::process::Command;

use pretty_assertions::assert_eq;

use super::*;

#[test]
fn registered_provider_key_is_removed_even_after_policy_overrides() {
    register_non_inheritable_env_var("CUTEX_TEST_PROVIDER_API_KEY");
    register_non_inheritable_env_var("CUTEX_TEST_PROVIDER_API_KEY");
    register_non_inheritable_env_var(" ");
    assert!(is_non_inheritable_env_var("cutex_test_provider_api_key"));

    let vars = [
        ("SAFE".to_string(), "inherited".to_string()),
        (
            "CUTEX_TEST_PROVIDER_API_KEY".to_string(),
            "synthetic-key".to_string(),
        ),
    ];
    let policy = ShellEnvironmentPolicy {
        inherit: ShellEnvironmentPolicyInherit::All,
        ignore_default_excludes: true,
        r#set: HashMap::from([("CUTEX_TEST_PROVIDER_API_KEY".to_string(), String::new())]),
        ..Default::default()
    };
    assert_eq!(
        populate_env(vars, &policy, /*thread_id*/ None),
        HashMap::from([("SAFE".to_string(), "inherited".to_string())])
    );

    let mut command = Command::new("true");
    command.env("CUTEX_TEST_PROVIDER_API_KEY", "synthetic-key");
    scrub_non_inheritable_env_vars(&mut command);
    assert!(
        command
            .get_envs()
            .all(|(name, value)| name != "CUTEX_TEST_PROVIDER_API_KEY" || value.is_none())
    );
}
