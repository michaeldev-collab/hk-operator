//! Pure composer rotation / lock / stack logic (no clipboard or ydotool).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub fn default_separator() -> String {
    " ".into()
}

pub fn default_timeout_ms() -> u64 {
    4000
}

pub fn default_reset_on() -> Vec<String> {
    vec!["timeout".into(), "explicitClear".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerConfig {
    pub commands: Vec<String>,
    #[serde(default = "default_separator")]
    pub separator: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_reset_on")]
    pub reset_on: Vec<String>,
}

impl Default for ComposerConfig {
    fn default() -> Self {
        Self {
            // Public portfolio defaults — no private board slash names.
            commands: vec!["/help".into(), "/review".into(), "/plan".into()],
            separator: default_separator(),
            timeout_ms: default_timeout_ms(),
            reset_on: default_reset_on(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ComposerRuntime {
    /// Current preview index while picking
    pub index: HashMap<String, usize>,
    /// Last press time
    pub last_press: HashMap<String, Instant>,
    /// Bump to cancel a pending "pause = lock in" timer
    pub generation: HashMap<String, u64>,
    /// True while rotating before a pause lock-in
    pub picking: HashMap<String, bool>,
    /// How many commands locked into this compose session (for separator)
    pub committed: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerPress {
    pub text: String,
    pub token: String,
    pub idx: usize,
    pub len: usize,
    pub timeout_ms: u64,
    /// Generation to pass to [`try_lock_composer`] after the pause timer.
    pub generation: u64,
    /// True when this press replaced a previous preview (caller may undo).
    pub replaced_preview: bool,
}

/// Validate composer id + config before mutating runtime.
pub fn composer_precheck(id: &str, cfg: Option<&ComposerConfig>) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("composer action needs a composer id in value (e.g. ai)".into());
    }
    let cfg = cfg.ok_or_else(|| format!("unknown composer \"{id}\""))?;
    if cfg.commands.is_empty() {
        return Err(format!("composer \"{id}\" has no commands"));
    }
    Ok(())
}

/// Apply one composer press. Side-effect free aside from `runtime` mutation.
pub fn apply_composer_press(
    id: &str,
    cfg: &ComposerConfig,
    runtime: &mut ComposerRuntime,
    now: Instant,
) -> Result<ComposerPress, String> {
    composer_precheck(id, Some(cfg))?;
    let id = id.trim();
    let len = cfg.commands.len();
    let timeout_ms = cfg.timeout_ms.max(500);
    let timeout = Duration::from_millis(timeout_ms);
    let was_picking = runtime.picking.get(id).copied().unwrap_or(false);

    // Very long idle → fresh compose session
    if let Some(last) = runtime.last_press.get(id) {
        if now.duration_since(*last) >= timeout.saturating_mul(3) {
            runtime.committed.insert(id.to_string(), 0);
            runtime.picking.insert(id.to_string(), false);
            runtime.index.insert(id.to_string(), 0);
        }
    }
    let committed = runtime.committed.get(id).copied().unwrap_or(0);

    let (idx, replaced_preview) = if was_picking {
        let cur = runtime.index.get(id).copied().unwrap_or(0) % len;
        ((cur + 1) % len, true)
    } else {
        (0, false)
    };

    let token = cfg.commands[idx].clone();
    let text = if committed > 0 {
        format!("{}{}", cfg.separator, token)
    } else {
        token.clone()
    };

    runtime.index.insert(id.to_string(), idx);
    runtime.picking.insert(id.to_string(), true);
    runtime.last_press.insert(id.to_string(), now);
    let gen = runtime.generation.entry(id.to_string()).or_insert(0);
    *gen += 1;
    let generation = *gen;

    Ok(ComposerPress {
        text,
        token,
        idx,
        len,
        timeout_ms,
        generation,
        replaced_preview,
    })
}

/// After idle pause: lock current preview if generation still matches.
/// Returns true when a lock occurred.
pub fn try_lock_composer(runtime: &mut ComposerRuntime, id: &str, expected_gen: u64) -> bool {
    if runtime.generation.get(id).copied().unwrap_or(0) == expected_gen
        && runtime.picking.get(id).copied().unwrap_or(false)
    {
        let n = runtime.committed.get(id).copied().unwrap_or(0) + 1;
        runtime.committed.insert(id.to_string(), n);
        runtime.picking.insert(id.to_string(), false);
        runtime.index.insert(id.to_string(), 0);
        true
    } else {
        false
    }
}

pub fn reset_composer_runtime(runtime: &mut ComposerRuntime, composer_id: Option<&str>) {
    if let Some(id) = composer_id {
        runtime.index.remove(id);
        runtime.last_press.remove(id);
        runtime.generation.remove(id);
        runtime.picking.remove(id);
        runtime.committed.remove(id);
    } else {
        *runtime = ComposerRuntime::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(commands: &[&str], timeout_ms: u64) -> ComposerConfig {
        ComposerConfig {
            commands: commands.iter().map(|s| (*s).to_string()).collect(),
            separator: " ".into(),
            timeout_ms,
            reset_on: default_reset_on(),
        }
    }

    #[test]
    fn first_press_starts_at_index_zero_without_separator() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help", "/review", "/plan"], 4000);
        let now = Instant::now();
        let press = apply_composer_press("ai", &c, &mut rt, now).unwrap();
        assert_eq!(press.token, "/help");
        assert_eq!(press.text, "/help");
        assert_eq!(press.idx, 0);
        assert!(!press.replaced_preview);
        assert!(rt.picking.get("ai").copied().unwrap_or(false));
    }

    #[test]
    fn second_press_while_picking_rotates() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help", "/review", "/plan"], 4000);
        let t0 = Instant::now();
        apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        let press = apply_composer_press("ai", &c, &mut rt, t0 + Duration::from_millis(10)).unwrap();
        assert_eq!(press.token, "/review");
        assert_eq!(press.idx, 1);
        assert!(press.replaced_preview);
    }

    #[test]
    fn wrap_from_last_to_first() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b"], 1000);
        let t0 = Instant::now();
        apply_composer_press("ai", &c, &mut rt, t0).unwrap(); // /a
        apply_composer_press("ai", &c, &mut rt, t0).unwrap(); // /b
        let press = apply_composer_press("ai", &c, &mut rt, t0).unwrap(); // wrap
        assert_eq!(press.token, "/a");
        assert_eq!(press.idx, 0);
    }

    #[test]
    fn lock_then_stack_prepends_separator() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help", "/review"], 500);
        let t0 = Instant::now();
        let p1 = apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        assert!(try_lock_composer(&mut rt, "ai", p1.generation));
        assert_eq!(rt.committed.get("ai").copied(), Some(1));
        assert!(!rt.picking.get("ai").copied().unwrap_or(true));

        let p2 = apply_composer_press("ai", &c, &mut rt, t0 + Duration::from_millis(10)).unwrap();
        assert_eq!(p2.text, " /help");
        assert_eq!(p2.token, "/help");
    }

    #[test]
    fn stale_generation_does_not_lock() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help"], 500);
        let t0 = Instant::now();
        let p1 = apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        let _p2 = apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        assert!(!try_lock_composer(&mut rt, "ai", p1.generation));
    }

    #[test]
    fn long_idle_resets_committed_session() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help", "/review"], 500);
        let t0 = Instant::now();
        let p1 = apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        assert!(try_lock_composer(&mut rt, "ai", p1.generation));
        // 3 × timeout = 1500ms
        let later = t0 + Duration::from_millis(1500);
        let press = apply_composer_press("ai", &c, &mut rt, later).unwrap();
        assert_eq!(press.text, "/help"); // no separator — session reset
        assert_eq!(rt.committed.get("ai").copied().unwrap_or(0), 0);
    }

    #[test]
    fn timeout_ms_floored_at_500() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help"], 100);
        let press = apply_composer_press("ai", &c, &mut rt, Instant::now()).unwrap();
        assert_eq!(press.timeout_ms, 500);
    }

    #[test]
    fn empty_id_and_unknown_composer_err() {
        let c = cfg(&["/help"], 4000);
        assert!(composer_precheck("  ", Some(&c)).is_err());
        assert!(composer_precheck("ai", None).is_err());
        let empty = ComposerConfig {
            commands: vec![],
            ..ComposerConfig::default()
        };
        assert!(composer_precheck("ai", Some(&empty)).is_err());
    }

    #[test]
    fn reset_clears_one_or_all() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/help"], 500);
        apply_composer_press("ai", &c, &mut rt, Instant::now()).unwrap();
        apply_composer_press("other", &c, &mut rt, Instant::now()).unwrap();
        reset_composer_runtime(&mut rt, Some("ai"));
        assert!(!rt.picking.contains_key("ai"));
        assert!(rt.picking.contains_key("other"));
        reset_composer_runtime(&mut rt, None);
        assert!(rt.picking.is_empty());
    }
}
