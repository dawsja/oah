//! Provider environment values (API keys) in the OS keychain: Windows Credential Manager, or
//! the Secret Service on Linux. settings.json keeps only the variable names.

use oah_core::settings::{AgentKind, EnvVar, Settings};

const SERVICE: &str = "dev.oah.app";

fn entry(agent: AgentKind, key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{agent:?}/{key}").to_lowercase()).map_err(|e| e.to_string())
}

/// Moves every typed value into the keychain and blanks it in `settings` (values already in
/// the keychain stay). Variables removed from the list are removed from the keychain.
pub fn stash(settings: &mut Settings, before: &Settings) -> Result<(), String> {
    for kind in AgentKind::ALL {
        let old: Vec<String> = before.providers.get(kind).env.iter().filter(|v| v.in_keychain).map(|v| v.key.clone()).collect();
        let vars = &mut settings.providers.get_mut(kind).env;
        for v in vars.iter_mut().filter(|v| !v.key.trim().is_empty()) {
            if !v.value.is_empty() {
                entry(kind, &v.key)?.set_password(&v.value).map_err(|e| format!("couldn't save {} in the keychain: {e}", v.key))?;
                v.value.clear();
                v.in_keychain = true;
            }
        }
        for k in old.iter().filter(|k| !vars.iter().any(|v| &v.key == *k)) {
            if let Ok(e) = entry(kind, k) {
                let _ = e.delete_credential();
            }
        }
    }
    Ok(())
}

/// The environment to start `kind` with: names from settings, values from the keychain.
pub fn resolve(kind: AgentKind, vars: &[EnvVar]) -> Vec<EnvVar> {
    vars.iter()
        .filter(|v| !v.key.trim().is_empty())
        .map(|v| {
            let value = if v.in_keychain {
                entry(kind, &v.key).and_then(|e| e.get_password().map_err(|e| e.to_string())).unwrap_or_default()
            } else {
                v.value.clone()
            };
            EnvVar { key: v.key.clone(), value, in_keychain: false }
        })
        .collect()
}
