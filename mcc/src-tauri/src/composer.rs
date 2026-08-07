//! Composer rotation / space-commit / stack logic (no clipboard or ydotool).
//!
//! Pad **double-tap** = start or rotate the current preview token.
//! Single tap is ignored (avoids accidental stack/rotate on slow presses).
//! Spacebar (host key) = commit; next double-tap starts the next stacked pick.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub fn default_separator() -> String {
    " ".into()
}

/// Kept for store/UI compat — idle abandon is disabled (B4 clears sessions).
pub fn default_timeout_ms() -> u64 {
    60_000
}

/// Max gap between two pad presses to count as one double-tap.
pub fn default_double_tap_ms() -> u64 {
    400
}

pub fn default_reset_on() -> Vec<String> {
    vec!["explicitClear".into(), "space".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerConfig {
    pub commands: Vec<String>,
    #[serde(default = "default_separator")]
    pub separator: String,
    /// Legacy field (ignored). New loop is explicit: P3 B4 / composer-reset.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_reset_on")]
    pub reset_on: Vec<String>,
}

impl Default for ComposerConfig {
    fn default() -> Self {
        Self {
            commands: vec!["/help".into(), "/review".into(), "/plan".into()],
            separator: default_separator(),
            timeout_ms: default_timeout_ms(),
            reset_on: default_reset_on(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ComposerRuntime {
    pub index: HashMap<String, usize>,
    pub last_press: HashMap<String, Instant>,
    /// First tap of a potential double-tap (not yet fired).
    pub last_tap: HashMap<String, Instant>,
    pub picking: HashMap<String, bool>,
    pub committed: HashMap<String, usize>,
    /// Preview token currently owned for this composer.
    pub last_text: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerPress {
    pub text: String,
    pub token: String,
    pub idx: usize,
    pub len: usize,
    pub timeout_ms: u64,
    pub replaced_preview: bool,
    /// FSM hint: prior preview length when rotating; 0 on a fresh pick.
    pub erase_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapOutcome {
    /// First tap of a double — do not touch the field.
    Arming,
    /// Confirmed double-tap — start or rotate.
    Fired(ComposerPress),
}

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

/// Pad tap gate: only a quick double-tap runs the composer.
pub fn note_composer_tap(
    id: &str,
    cfg: &ComposerConfig,
    runtime: &mut ComposerRuntime,
    now: Instant,
    double_tap_ms: u64,
) -> Result<TapOutcome, String> {
    composer_precheck(id, Some(cfg))?;
    let id = id.trim();
    let window = Duration::from_millis(double_tap_ms.clamp(150, 900));

    if let Some(first) = runtime.last_tap.get(id).copied() {
        if now.duration_since(first) <= window {
            runtime.last_tap.remove(id);
            let press = apply_composer_press(id, cfg, runtime, now)?;
            return Ok(TapOutcome::Fired(press));
        }
    }

    runtime.last_tap.insert(id.to_string(), now);
    Ok(TapOutcome::Arming)
}

pub fn apply_composer_press(
    id: &str,
    cfg: &ComposerConfig,
    runtime: &mut ComposerRuntime,
    now: Instant,
) -> Result<ComposerPress, String> {
    composer_precheck(id, Some(cfg))?;
    let id = id.trim();
    let len = cfg.commands.len();
    let timeout_ms = cfg.timeout_ms.max(5_000);
    // No idle abandon — sitting between rotates must not wipe locked chips.
    // New loop is explicit via composer-reset (P3 B4).

    let was_picking = runtime.picking.get(id).copied().unwrap_or(false);

    let (idx, replaced_preview) = if was_picking {
        let cur = runtime.index.get(id).copied().unwrap_or(0) % len;
        ((cur + 1) % len, true)
    } else {
        (0, false)
    };

    let token = cfg.commands[idx].clone();
    let text = token.clone();

    let erase_chars = if replaced_preview {
        runtime
            .last_text
            .get(id)
            .map(|t| t.chars().count())
            .unwrap_or(0)
    } else {
        0
    };
    runtime.last_text.insert(id.to_string(), text.clone());
    runtime.index.insert(id.to_string(), idx);
    runtime.picking.insert(id.to_string(), true);
    runtime.last_press.insert(id.to_string(), now);

    Ok(ComposerPress {
        text,
        token,
        idx,
        len,
        timeout_ms,
        replaced_preview,
        erase_chars,
    })
}

/// Spacebar (or explicit commit): keep the current preview; next double-tap stacks.
pub fn commit_composer(runtime: &mut ComposerRuntime, id: &str) -> bool {
    if !runtime.picking.get(id).copied().unwrap_or(false) {
        return false;
    }
    let n = runtime.committed.get(id).copied().unwrap_or(0) + 1;
    runtime.committed.insert(id.to_string(), n);
    runtime.picking.insert(id.to_string(), false);
    runtime.index.insert(id.to_string(), 0);
    runtime.last_text.remove(id);
    runtime.last_tap.remove(id);
    true
}

pub fn commit_all_picking(runtime: &mut ComposerRuntime) -> Vec<String> {
    let ids: Vec<String> = runtime
        .picking
        .iter()
        .filter(|(_, v)| **v)
        .map(|(k, _)| k.clone())
        .collect();
    let mut committed = Vec::new();
    for id in ids {
        if commit_composer(runtime, &id) {
            committed.push(id);
        }
    }
    committed
}

pub fn any_picking(runtime: &ComposerRuntime) -> bool {
    runtime.picking.values().any(|v| *v)
}

pub fn reset_composer_runtime(runtime: &mut ComposerRuntime, composer_id: Option<&str>) {
    if let Some(id) = composer_id {
        runtime.index.remove(id);
        runtime.last_press.remove(id);
        runtime.last_tap.remove(id);
        runtime.picking.remove(id);
        runtime.committed.remove(id);
        runtime.last_text.remove(id);
    } else {
        *runtime = ComposerRuntime::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(commands: &[&str]) -> ComposerConfig {
        ComposerConfig {
            commands: commands.iter().map(|s| (*s).to_string()).collect(),
            separator: " ".into(),
            timeout_ms: 60_000,
            reset_on: default_reset_on(),
        }
    }

    #[test]
    fn single_tap_arms_without_firing() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b"]);
        let t0 = Instant::now();
        let out = note_composer_tap("ai", &c, &mut rt, t0, 400).unwrap();
        assert_eq!(out, TapOutcome::Arming);
        assert!(!rt.picking.get("ai").copied().unwrap_or(false));
    }

    #[test]
    fn double_tap_starts_then_rotates() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b", "/c"]);
        let t0 = Instant::now();
        assert!(matches!(
            note_composer_tap("ai", &c, &mut rt, t0, 400).unwrap(),
            TapOutcome::Arming
        ));
        let TapOutcome::Fired(p1) =
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(120), 400).unwrap()
        else {
            panic!("expected fire");
        };
        assert_eq!(p1.text, "/a");

        assert!(matches!(
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(200), 400).unwrap(),
            TapOutcome::Arming
        ));
        let TapOutcome::Fired(p2) =
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(320), 400).unwrap()
        else {
            panic!("expected fire");
        };
        assert_eq!(p2.text, "/b");
        assert!(p2.replaced_preview);
    }

    #[test]
    fn slow_second_tap_is_new_arm_not_rotate() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b"]);
        let t0 = Instant::now();
        note_composer_tap("ai", &c, &mut rt, t0, 400).unwrap();
        let out =
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(500), 400).unwrap();
        assert_eq!(out, TapOutcome::Arming);
        assert!(!rt.picking.get("ai").copied().unwrap_or(false));
    }

    #[test]
    fn space_then_double_tap_stacks() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b"]);
        let t0 = Instant::now();
        note_composer_tap("ai", &c, &mut rt, t0, 400).unwrap();
        let TapOutcome::Fired(_) =
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(100), 400).unwrap()
        else {
            panic!("fire");
        };
        assert!(commit_composer(&mut rt, "ai"));

        note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(200), 400).unwrap();
        let TapOutcome::Fired(p) =
            note_composer_tap("ai", &c, &mut rt, t0 + Duration::from_millis(300), 400).unwrap()
        else {
            panic!("fire");
        };
        assert_eq!(p.text, "/a");
        assert!(!p.replaced_preview);
    }

    #[test]
    fn apply_composer_press_still_rotates_directly() {
        let mut rt = ComposerRuntime::default();
        let c = cfg(&["/a", "/b", "/c"]);
        let t0 = Instant::now();
        let p1 = apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        assert_eq!(p1.text, "/a");
        let p2 = apply_composer_press("ai", &c, &mut rt, t0 + Duration::from_secs(2)).unwrap();
        assert_eq!(p2.text, "/b");
    }

    #[test]
    fn long_idle_does_not_reset_session() {
        let mut rt = ComposerRuntime::default();
        let mut c = cfg(&["/a", "/b"]);
        c.timeout_ms = 5_000;
        let t0 = Instant::now();
        apply_composer_press("ai", &c, &mut rt, t0).unwrap();
        assert!(commit_composer(&mut rt, "ai"));
        // Stack after a long pause — must still be a fresh pick after commit,
        // not a wiped writer session from idle abandon.
        let p = apply_composer_press("ai", &c, &mut rt, t0 + Duration::from_secs(120)).unwrap();
        assert!(!p.replaced_preview);
        assert_eq!(p.text, "/a");
        assert!(rt.picking.get("ai").copied().unwrap_or(false));
    }
}
