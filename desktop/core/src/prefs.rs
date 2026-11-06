//! Desktop-only preferences in `<oah home>/desktop.json`: the last workspace and
//! panel layout. Unknown keys are kept, so newer app versions can add fields.

use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

pub fn load(path: &Path) -> Map<String, Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

/// Merge `patch` into the saved preferences (a `null` value removes a key) and
/// write them atomically.
pub fn update(path: &Path, patch: Map<String, Value>) -> std::io::Result<Map<String, Value>> {
    let mut prefs = load(path);
    for (k, v) in patch {
        if v.is_null() {
            prefs.remove(&k);
        } else {
            prefs.insert(k, v);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(prefs.clone()))?)?;
    fs::rename(&tmp, path)?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/desktop.json");
        assert!(load(&p).is_empty());
        let m = |v: Value| v.as_object().unwrap().clone();
        update(&p, m(json!({"lastCwd": "/a", "inspector": true}))).unwrap();
        let got = update(&p, m(json!({"lastCwd": "/b", "inspector": null}))).unwrap();
        assert_eq!(Value::Object(got), json!({"lastCwd": "/b"}));
        assert_eq!(Value::Object(load(&p)), json!({"lastCwd": "/b"}));
    }
}
