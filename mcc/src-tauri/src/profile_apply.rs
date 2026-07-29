//! Profile apply rules (P3-02) — pure store mutation helpers.

use crate::dispatch::AllowedCommands;
use std::collections::HashSet;

/// Result of applying a portable profile without importing shell allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileApplyStats {
    /// How many allowlist entries were listed in the profile (ignored).
    pub profile_allowlist_ignored: usize,
    /// How many live allowlist ids remain after intersecting with new action ids.
    pub retained_allowlist: usize,
}

/// Apply portable profile fields into a live allowlist-aware store view.
///
/// **P3-02:** `profile_allowed_commands` is never installed. Existing live
/// allowlist entries (id → value fingerprint) are kept only when the action id
/// still exists after apply. Fingerprints are left unchanged so a replaced
/// value fails the P3-03 gate until re-approval.
pub fn merge_allowlist_after_profile(
    live_allowed: &mut AllowedCommands,
    new_action_ids: &HashSet<String>,
    profile_allowlist_entry_count: usize,
) -> ProfileApplyStats {
    live_allowed.retain(|id, _fp| new_action_ids.contains(id));
    ProfileApplyStats {
        profile_allowlist_ignored: profile_allowlist_entry_count,
        retained_allowlist: live_allowed.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::command_value_fingerprint;

    #[test]
    fn does_not_install_profile_allowlist() {
        let mut live = AllowedCommands::new();
        live.insert("keep".into(), command_value_fingerprint("echo keep"));
        let actions: HashSet<String> = ["keep", "new"].iter().map(|s| (*s).into()).collect();
        let stats = merge_allowlist_after_profile(&mut live, &actions, 2);
        assert_eq!(stats.profile_allowlist_ignored, 2);
        assert!(live.contains_key("keep"));
        assert!(!live.contains_key("evil"));
        assert!(!live.contains_key("new"));
        assert_eq!(stats.retained_allowlist, 1);
    }

    #[test]
    fn drops_allowlist_ids_removed_from_actions() {
        let mut live = AllowedCommands::new();
        live.insert("a".into(), "1".into());
        live.insert("b".into(), "2".into());
        let actions: HashSet<String> = ["a"].iter().map(|s| (*s).into()).collect();
        merge_allowlist_after_profile(&mut live, &actions, 0);
        assert_eq!(live.len(), 1);
        assert!(live.contains_key("a"));
    }

    #[test]
    fn empty_live_stays_empty_even_if_profile_reported_entries() {
        let mut live = AllowedCommands::new();
        let actions: HashSet<String> = ["x"].iter().map(|s| (*s).into()).collect();
        let stats = merge_allowlist_after_profile(&mut live, &actions, 1);
        assert!(live.is_empty());
        assert_eq!(stats.profile_allowlist_ignored, 1);
    }
}
