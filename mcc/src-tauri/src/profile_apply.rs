//! Profile apply rules (P3-02) — pure store mutation helpers.

use std::collections::HashSet;

/// Result of applying a portable profile without importing shell allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileApplyStats {
    /// How many action ids were listed in the profile's `allowedCommands` (ignored).
    pub profile_allowlist_ignored: usize,
    /// How many live allowlist ids remain after intersecting with new action ids.
    pub retained_allowlist: usize,
}

/// Apply portable profile fields into a live allowlist-aware store view.
///
/// **P3-02:** `profile_allowed_commands` is never installed. Existing live
/// allowlist entries are kept only when the action id still exists after apply.
pub fn merge_allowlist_after_profile(
    live_allowed: &mut HashSet<String>,
    new_action_ids: &HashSet<String>,
    profile_allowed_commands: &HashSet<String>,
) -> ProfileApplyStats {
    let profile_allowlist_ignored = profile_allowed_commands.len();
    live_allowed.retain(|id| new_action_ids.contains(id));
    ProfileApplyStats {
        profile_allowlist_ignored,
        retained_allowlist: live_allowed.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_install_profile_allowlist() {
        let mut live: HashSet<String> = ["keep"].iter().map(|s| (*s).into()).collect();
        let actions: HashSet<String> = ["keep", "new"].iter().map(|s| (*s).into()).collect();
        let profile: HashSet<String> = ["evil", "new"].iter().map(|s| (*s).into()).collect();
        let stats = merge_allowlist_after_profile(&mut live, &actions, &profile);
        assert_eq!(stats.profile_allowlist_ignored, 2);
        assert!(live.contains("keep"));
        assert!(!live.contains("evil"));
        assert!(!live.contains("new")); // was not previously approved
        assert_eq!(stats.retained_allowlist, 1);
    }

    #[test]
    fn drops_allowlist_ids_removed_from_actions() {
        let mut live: HashSet<String> = ["a", "b"].iter().map(|s| (*s).into()).collect();
        let actions: HashSet<String> = ["a"].iter().map(|s| (*s).into()).collect();
        let profile: HashSet<String> = HashSet::new();
        merge_allowlist_after_profile(&mut live, &actions, &profile);
        assert_eq!(live, ["a"].iter().map(|s| (*s).into()).collect());
    }

    #[test]
    fn empty_live_stays_empty_even_if_profile_has_allowlist() {
        let mut live = HashSet::new();
        let actions: HashSet<String> = ["x"].iter().map(|s| (*s).into()).collect();
        let profile: HashSet<String> = ["x"].iter().map(|s| (*s).into()).collect();
        let stats = merge_allowlist_after_profile(&mut live, &actions, &profile);
        assert!(live.is_empty());
        assert_eq!(stats.profile_allowlist_ignored, 1);
    }
}
