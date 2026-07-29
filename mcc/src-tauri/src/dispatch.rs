//! Pure dispatch gates (no shell, open, or clipboard side effects).

use std::collections::{HashMap, HashSet};

/// Match host URL gate used by `execute_action` (prefix check, not regex).
pub fn url_scheme_allowed(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

pub fn url_gate(value: &str) -> Result<(), String> {
    if url_scheme_allowed(value) {
        Ok(())
    } else {
        Err("Blocked: not an http(s) URL".into())
    }
}

pub fn command_gate(
    allowed: &HashSet<String>,
    action_id: &str,
    action_name: &str,
) -> Result<(), String> {
    if allowed.contains(action_id) {
        Ok(())
    } else {
        Err(format!(
            "Command \"{action_name}\" not allowed yet — approve it in the UI first"
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
        assert!(!url_scheme_allowed("ftp://example.com"));
        assert!(!url_scheme_allowed("example.com"));
        assert!(!url_scheme_allowed(""));
        assert!(!url_scheme_allowed("HTTPS://example.com")); // Rust gate is case-sensitive prefix
        assert_eq!(
            url_gate("ftp://x").unwrap_err(),
            "Blocked: not an http(s) URL"
        );
    }

    #[test]
    fn command_allowlist_by_action_id() {
        let mut allowed = HashSet::new();
        assert!(command_gate(&allowed, "a1", "Run ls").is_err());
        allowed.insert("a1".into());
        assert!(command_gate(&allowed, "a1", "Run ls").is_ok());
        assert!(command_gate(&allowed, "a2", "Other").is_err());
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
