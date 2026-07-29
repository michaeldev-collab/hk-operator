//! Pure dispatch gates (no shell, open, or clipboard side effects).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Match host URL gate used by `execute_action` (prefix check, not regex).
/// Scheme compare is ASCII-case-insensitive and trims surrounding whitespace
/// so Rust matches the JS `/^https?:\/\//i` validator (P3-09).
pub fn url_scheme_allowed(value: &str) -> bool {
    let v = value.trim().as_bytes();
    // Check `https://` before `http://` — the latter is a prefix of the former.
    (v.len() >= 8 && v[..8].eq_ignore_ascii_case(b"https://"))
        || (v.len() >= 7 && v[..7].eq_ignore_ascii_case(b"http://"))
}

pub fn url_gate(value: &str) -> Result<(), String> {
    if url_scheme_allowed(value) {
        Ok(())
    } else {
        Err("Blocked: not an http(s) URL".into())
    }
}

/// Stable FNV-1a 64-bit fingerprint of a command action value (P3-03).
/// Must stay in sync with `commandValueFingerprint` in `mcc/src/lib.js`.
pub fn command_value_fingerprint(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in value.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Allowlist is action-id → fingerprint of the approved `value`.
pub type AllowedCommands = HashMap<String, String>;

pub fn parse_allowed_commands_json(v: &Value) -> AllowedCommands {
    match v {
        Value::Object(map) => map
            .iter()
            .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        // Legacy id-only arrays: drop — require re-approval under P3-03.
        Value::Array(_) | Value::Null => AllowedCommands::new(),
        _ => AllowedCommands::new(),
    }
}

pub fn deserialize_allowed_commands<'de, D>(deserializer: D) -> Result<AllowedCommands, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    Ok(parse_allowed_commands_json(&v))
}

pub fn serialize_allowed_commands<S>(
    value: &AllowedCommands,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.serialize(serializer)
}

/// Count portable allowlist entries for P3-02 ignore stats (array or object).
pub fn portable_allowlist_entry_count(v: &Value) -> usize {
    match v {
        Value::Array(a) => a.len(),
        Value::Object(m) => m.len(),
        _ => 0,
    }
}

pub fn command_gate(
    allowed: &AllowedCommands,
    action_id: &str,
    action_name: &str,
    action_value: &str,
) -> Result<(), String> {
    let Some(approved_fp) = allowed.get(action_id) else {
        return Err(format!(
            "Command \"{action_name}\" not allowed yet — approve it in the UI first"
        ));
    };
    let current = command_value_fingerprint(action_value);
    if approved_fp == &current {
        Ok(())
    } else {
        Err(format!(
            "Command \"{action_name}\" value changed since approval — re-approve it in the UI"
        ))
    }
}

/// Known executable action types on the host dispatcher.
pub fn known_action_type(type_: &str) -> bool {
    matches!(
        type_,
        "url" | "path" | "prompt" | "note" | "composer" | "command"
    )
}

pub fn unknown_type_err(type_: &str) -> String {
    format!("unknown action type: {type_}")
}

/// Resolve a pad binding key to an action id, then validate the id exists.
pub fn resolve_binding_action_id(
    bindings: &HashMap<String, String>,
    action_ids: &HashSet<String>,
    key: &str,
) -> Result<String, String> {
    let id = bindings
        .get(key)
        .cloned()
        .ok_or_else(|| format!("no binding for slot {key}"))?;
    if action_ids.contains(&id) {
        Ok(id)
    } else {
        Err(format!("binding {key} points to missing action"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_allows_http_https_only() {
        assert!(url_scheme_allowed("http://example.com"));
        assert!(url_scheme_allowed("https://example.com/x"));
        assert!(url_scheme_allowed("HTTPS://example.com"));
        assert!(url_scheme_allowed("  HtTp://example.com/x  "));
        assert!(!url_scheme_allowed("ftp://example.com"));
        assert!(!url_scheme_allowed("example.com"));
        assert!(!url_scheme_allowed(""));
        assert!(!url_scheme_allowed("javascript:alert(1)"));
        assert_eq!(
            url_gate("ftp://x").unwrap_err(),
            "Blocked: not an http(s) URL"
        );
    }

    #[test]
    fn fingerprint_stable_and_value_sensitive() {
        let a = command_value_fingerprint("ls -la");
        let b = command_value_fingerprint("ls -la");
        let c = command_value_fingerprint("ls -la /tmp");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
        assert_eq!(command_value_fingerprint("ls"), "08ad4d07b5541ae8");
    }

    #[test]
    fn command_allowlist_binds_value_fingerprint() {
        let mut allowed = AllowedCommands::new();
        let fp = command_value_fingerprint("ls");
        assert!(command_gate(&allowed, "a1", "Run ls", "ls").is_err());
        allowed.insert("a1".into(), fp.clone());
        assert!(command_gate(&allowed, "a1", "Run ls", "ls").is_ok());
        let err = command_gate(&allowed, "a1", "Run ls", "rm -rf /").unwrap_err();
        assert!(err.contains("value changed"));
        assert!(command_gate(&allowed, "a2", "Other", "true").is_err());
    }

    #[test]
    fn legacy_array_allowed_commands_parse_empty() {
        let v = serde_json::json!(["a1", "a2"]);
        assert!(parse_allowed_commands_json(&v).is_empty());
        let v2 = serde_json::json!({"a1": "deadbeefcafebabe"});
        assert_eq!(
            parse_allowed_commands_json(&v2).get("a1").map(String::as_str),
            Some("deadbeefcafebabe")
        );
    }

    #[test]
    fn unknown_types_rejected() {
        assert!(known_action_type("url"));
        assert!(known_action_type("composer"));
        assert!(!known_action_type("boom"));
        assert_eq!(unknown_type_err("boom"), "unknown action type: boom");
    }

    #[test]
    fn binding_resolution_messages() {
        let mut bindings = HashMap::new();
        let mut ids = HashSet::new();
        assert_eq!(
            resolve_binding_action_id(&bindings, &ids, "2-0").unwrap_err(),
            "no binding for slot 2-0"
        );
        bindings.insert("2-0".into(), "missing".into());
        assert_eq!(
            resolve_binding_action_id(&bindings, &ids, "2-0").unwrap_err(),
            "binding 2-0 points to missing action"
        );
        ids.insert("missing".into());
        assert_eq!(
            resolve_binding_action_id(&bindings, &ids, "2-0").unwrap(),
            "missing"
        );
    }
}
