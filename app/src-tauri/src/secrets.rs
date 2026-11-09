//! Provider environment values (API keys) in the OS keychain: Windows Credential Manager, or
//! the Secret Service on Linux. settings.json keeps only the variable names.

use oah_core::settings::{AgentKind, EnvVar, Settings};

const SERVICE: &str = "dev.oah.app";

/// Whose variables: `claude`, `codex` or `opencode`.
pub fn scope(kind: AgentKind) -> String {
    format!("{kind:?}").to_lowercase()
}

fn entry(scope: &str, key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{scope}/{key}")).map_err(|e| e.to_string())
}

fn stash_vars(scope: &str, vars: &mut [EnvVar], old: &[EnvVar]) -> Result<(), String> {
    for v in vars.iter_mut().filter(|v| !v.key.trim().is_empty()) {
        if !v.value.is_empty() {
            entry(scope, &v.key)?.set_password(&v.value).map_err(|e| format!("couldn't save {} in the keychain: {e}", v.key))?;
            v.value.clear();
            v.in_keychain = true;
        }
    }
    for k in old.iter().filter(|o| o.in_keychain && !vars.iter().any(|v| v.key == o.key)) {
        if let Ok(e) = entry(scope, &k.key) {
            let _ = e.delete_credential();
        }
    }
    Ok(())
}

/// Moves every typed value into the keychain and blanks it in `settings` (values already in
/// the keychain stay). Variables removed from a list are removed from the keychain.
pub fn stash(settings: &mut Settings, before: &Settings) -> Result<(), String> {
    for kind in AgentKind::ALL {
        let old = before.providers.get(kind).env.clone();
        stash_vars(&scope(kind), &mut settings.providers.get_mut(kind).env, &old)?;
    }
    Ok(())
}

/// The environment to start an agent with: names from settings, values from the keychain.
pub fn resolve(scope: &str, vars: &[EnvVar]) -> Vec<EnvVar> {
    vars.iter()
        .filter(|v| !v.key.trim().is_empty())
        .map(|v| {
            let value = if v.in_keychain {
                entry(scope, &v.key).and_then(|e| e.get_password().map_err(|e| e.to_string())).unwrap_or_default()
            } else {
                v.value.clone()
            };
            EnvVar { key: v.key.clone(), value, in_keychain: false }
        })
        .collect()
}
