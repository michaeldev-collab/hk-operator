//! Single-flight coalesced field writer for the slash composer.
//!
//! Watchdog = `on_screen`: the exact preview string last painted. Its
//! `chars().count()` is how many trailing chars to select/replace on rotate.
//! Command lengths come from the MCC composer list (whatever was pasted) —
//! we do **not** walk letter-by-letter until a space.
//!
//! Modes:
//! - **Fresh** (`locked` empty): Ctrl+A → Delete → paste preview
//! - **Stack** (`locked` non-empty): bulk-select last N preview chars → Delete
//!   → paste new preview only (Cursor chips / committed text stay intact)
//!
//! Focus safety: capture the active window when a composition session begins.
//! Every field mutation re-checks that window; on mismatch, abort without
//! emitting keystrokes and reset both the field writer and composer runtime.

use crate::composer::{reset_composer_runtime, ComposerRuntime};
use crate::ydotool_sock::ensure_ydotoold;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub text: String,
    /// 0 = fresh pick / append (ignore stale on_screen). >0 = rotate erase hint.
    pub erase_hint: usize,
    pub gen: u64,
}

struct WriterState {
    desired: Option<WriteRequest>,
    next_gen: u64,
    locked: String,
    /// Watchdog: last painted preview (length drives bulk select-on-rotate).
    on_screen: Option<String>,
    /// Active window id captured at composition start (`kwin:` / `x11:`).
    target_window: Option<String>,
}

pub struct FieldWriter {
    state: Mutex<WriterState>,
    notify: Notify,
    config_dir: Option<PathBuf>,
}

#[cfg(test)]
fn compose_field(locked: &str, preview: &str) -> String {
    format!("{locked}{preview}")
}

pub fn uses_full_rewrite(locked: &str) -> bool {
    locked.is_empty()
}

/// How many trailing chars the watchdog should select on rotate.
pub fn preview_erase_len(on_screen: Option<&str>) -> usize {
    on_screen.map(|s| s.chars().count()).unwrap_or(0)
}

/// Combine painter watchdog + FSM hint. Fresh picks (hint 0) never erase.
pub fn segment_erase_len(on_screen: Option<&str>, erase_hint: usize) -> usize {
    if erase_hint == 0 {
        return 0;
    }
    preview_erase_len(on_screen).max(erase_hint)
}

impl FieldWriter {
    pub fn new() -> Self {
        Self::with_config_dir(None)
    }

    pub fn with_config_dir(config_dir: Option<PathBuf>) -> Self {
        Self {
            state: Mutex::new(WriterState {
                desired: None,
                next_gen: 1,
                locked: String::new(),
                on_screen: None,
                target_window: None,
            }),
            notify: Notify::new(),
            config_dir,
        }
    }

    pub async fn request(&self, text: String, erase_hint: usize) -> u64 {
        let mut g = self.state.lock().await;
        let gen = g.next_gen;
        g.next_gen = g.next_gen.wrapping_add(1).max(1);
        g.desired = Some(WriteRequest {
            text,
            erase_hint,
            gen,
        });
        self.notify.notify_one();
        gen
    }

    pub async fn clear_preview(&self) {
        let mut g = self.state.lock().await;
        if let Some(preview) = g.on_screen.take() {
            g.locked.push_str(&preview);
            // Bookkeeping only — the Space key already typed the separator
            // into the field. One trailing space marks the stack boundary.
            g.locked.push(' ');
        }
        g.desired = None;
        eprintln!("[composer-write] committed → locked={:?}", g.locked);
    }

    pub async fn reset(&self) {
        let mut g = self.state.lock().await;
        g.locked.clear();
        g.on_screen = None;
        g.desired = None;
        g.target_window = None;
        eprintln!("[composer-write] reset");
    }

    /// Capture active window on first write of a session; reuse afterward.
    async fn capture_or_get_target(&self) -> Result<String, String> {
        let mut g = self.state.lock().await;
        if let Some(t) = g.target_window.clone() {
            return Ok(t);
        }
        let id = active_window_id()
            .ok_or_else(|| "focus: could not determine active window".to_string())?;
        eprintln!("[composer-write] target window={id}");
        g.target_window = Some(id.clone());
        Ok(id)
    }

    async fn take_desired(&self) -> Option<WriteRequest> {
        self.state.lock().await.desired.take()
    }

    async fn snapshot(&self) -> (String, Option<String>) {
        let g = self.state.lock().await;
        (g.locked.clone(), g.on_screen.clone())
    }

    async fn set_on_screen(&self, text: Option<String>) {
        self.state.lock().await.on_screen = text;
    }

    fn ydotool_socket(&self) -> PathBuf {
        ensure_ydotoold(self.config_dir.as_deref())
    }
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Best-effort active window id (KWin DBus, then xdotool).
fn active_window_id() -> Option<String> {
    if let Ok(out) = Command::new("qdbus6")
        .args([
            "org.kde.KWin",
            "/KWin",
            "org.kde.KWin.activeWindow",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && s != "0" {
                return Some(format!("kwin:{s}"));
            }
        }
    }
    if let Ok(out) = Command::new("xdotool")
        .args(["getactivewindow"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(format!("x11:{s}"));
            }
        }
    }
    None
}

fn verify_focus(expected: &str) -> Result<(), String> {
    match active_window_id() {
        Some(got) if got == expected => Ok(()),
        Some(got) => Err(format!("focus: expected {expected}, got {got}")),
        None => Err("focus: could not determine active window".into()),
    }
}

fn notify_focus_abort(detail: &str) {
    let _ = Command::new("notify-send")
        .args([
            "-a",
            "MCC Pad",
            "composer aborted",
            detail,
        ])
        .status();
}

fn ydotool_key(sock: &PathBuf, args: &[String]) -> Result<(), String> {
    let status = Command::new("ydotool")
        .env("YDOTOOL_SOCKET", sock)
        .args(args)
        .status()
        .map_err(|e| format!("ydotool key failed: {e}"))?;
    if !status.success() {
        return Err(format!("ydotool key exited {status}"));
    }
    Ok(())
}

fn chord_ctrl(sock: &PathBuf, key_code: u16) -> Result<(), String> {
    let k = key_code.to_string();
    ydotool_key(
        sock,
        &[
            "key".into(),
            "--key-delay=18".into(),
            "29:1".into(),
            format!("{k}:1"),
            format!("{k}:0"),
            "29:0".into(),
        ],
    )
}

fn tap_key(sock: &PathBuf, key_code: u16) -> Result<(), String> {
    let k = key_code.to_string();
    ydotool_key(
        sock,
        &[
            "key".into(),
            "--key-delay=15".into(),
            format!("{k}:1"),
            format!("{k}:0"),
        ],
    )
}

fn klipper_set(text: &str) -> bool {
    Command::new("qdbus6")
        .args([
            "org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.setClipboardContents",
            text,
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn arboard_set(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}

/// Hot-path clipboard: one Klipper set + short settle (no multi-attempt verify).
fn set_clipboard_fast(text: &str) -> Result<(), String> {
    if klipper_set(text) {
        sleep_ms(45);
        return Ok(());
    }
    arboard_set(text)?;
    sleep_ms(45);
    Ok(())
}

fn select_all_clear(sock: &PathBuf, target: &str) -> Result<(), String> {
    verify_focus(target)?;
    // Single Ctrl+A is enough when paced; double only on fresh-field first paint.
    chord_ctrl(sock, 30)?; // A
    sleep_ms(35);
    verify_focus(target)?;
    tap_key(sock, 111)?; // DELETE
    sleep_ms(35);
    Ok(())
}

fn paste_chord(sock: &PathBuf, target: &str) -> Result<(), String> {
    verify_focus(target)?;
    chord_ctrl(sock, 47)?; // V
    sleep_ms(50);
    Ok(())
}

fn type_text(sock: &PathBuf, target: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    verify_focus(target)?;
    let mut child = Command::new("ydotool")
        .env("YDOTOOL_SOCKET", sock)
        .args(["type", "--key-delay=5", "--escape=0", "-f", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ydotool type failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("ydotool type stdin: {e}"))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("ydotool type wait: {e}"))?;
    if !status.success() {
        return Err(format!("ydotool type exited {status}"));
    }
    sleep_ms(40);
    Ok(())
}

/// Delete exactly `n` trailing chars (watchdog length) in one ydotool call.
///
/// Shift+Left bursts were dropping events in Cursor/Chrome (left `/3`, `/3d`
/// stubs). Backspace × N at a moderate delay is slower than 5ms Left but much
/// more reliable — and still far faster than the old 40ms chunked path.
fn erase_trailing_chars(sock: &PathBuf, target: &str, n: usize) -> Result<(), String> {
    if n == 0 {
        return Ok(());
    }
    verify_focus(target)?;
    // KEY_BACKSPACE=14
    let mut args: Vec<String> = vec!["key".into(), "--key-delay=12".into()];
    for _ in 0..n {
        args.push("14:1".into());
        args.push("14:0".into());
    }
    ydotool_key(sock, &args)?;
    sleep_ms(35);
    Ok(())
}

fn insert_text(sock: &PathBuf, target: &str, text: &str) -> Result<(), String> {
    set_clipboard_fast(text)?;
    match paste_chord(sock, target) {
        Ok(()) => {
            eprintln!("[composer-write] paste ok len={}", text.len());
            Ok(())
        }
        Err(e) if e.starts_with("focus:") => Err(e),
        Err(e) => {
            eprintln!("[composer-write] paste failed ({e}) — type fallback");
            type_text(sock, target, text)
        }
    }
}

/// First token only — full field rewrite.
fn apply_full_rewrite(sock: &PathBuf, target: &str, preview: &str) -> Result<(), String> {
    verify_focus(target)?;
    set_clipboard_fast(preview)?;
    select_all_clear(sock, target)?;
    insert_text(sock, target, preview)
}

/// After Space commit — only touch the trailing preview segment.
fn apply_segment(
    sock: &PathBuf,
    target: &str,
    preview: &str,
    on_screen: Option<&str>,
    erase_hint: usize,
) -> Result<(), String> {
    let erase = segment_erase_len(on_screen, erase_hint);
    eprintln!(
        "[composer-write] segment erase={} (watchdog={:?} hint={}) → {:?}",
        erase, on_screen, erase_hint, preview
    );
    erase_trailing_chars(sock, target, erase)?;
    insert_text(sock, target, preview)
}

fn apply_once(
    sock: &PathBuf,
    target: &str,
    locked_empty: bool,
    preview: &str,
    on_screen: Option<String>,
    erase_hint: usize,
) -> Result<(), String> {
    if locked_empty {
        apply_full_rewrite(sock, target, preview)
    } else {
        apply_segment(sock, target, preview, on_screen.as_deref(), erase_hint)
    }
}

/// Reset field writer + composer FSM together (focus abort / session wipe).
async fn abort_composition_session(
    writer: &FieldWriter,
    runtime: &Mutex<ComposerRuntime>,
) {
    writer.reset().await;
    let mut rt = runtime.lock().await;
    reset_composer_runtime(&mut rt, None);
    eprintln!("[composer-write] aborted — writer + composer runtime cleared");
}

pub async fn writer_loop(
    writer: Arc<FieldWriter>,
    runtime: Arc<Mutex<ComposerRuntime>>,
) {
    loop {
        writer.notify.notified().await;
        loop {
            let Some(req) = writer.take_desired().await else {
                break;
            };
            // Tiny coalesce window so a double-tap's second fire wins.
            tokio::time::sleep(Duration::from_millis(15)).await;
            let req = writer.take_desired().await.unwrap_or(req);

            let target = match writer.capture_or_get_target().await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[composer-write] {e}");
                    notify_focus_abort(
                        "Could not lock target window — focus a text field and double-tap again.",
                    );
                    abort_composition_session(&writer, &runtime).await;
                    break;
                }
            };

            let gen = req.gen;
            let erase_hint = req.erase_hint;
            let (locked, on_screen) = writer.snapshot().await;
            let locked_empty = uses_full_rewrite(&locked);
            let preview = req.text.clone();
            let mode = if locked_empty { "full" } else { "segment" };
            let on_screen_clone = on_screen.clone();
            let sock = writer.ydotool_socket();
            let target_clone = target.clone();
            let started = std::time::Instant::now();
            let result = tokio::task::spawn_blocking(move || {
                apply_once(
                    &sock,
                    &target_clone,
                    locked_empty,
                    &preview,
                    on_screen_clone,
                    erase_hint,
                )
            })
            .await
            .map_err(|e| e.to_string());
            match result {
                Ok(Ok(())) => {
                    eprintln!(
                        "[composer-write] gen={} mode={} erase={} ms={} preview={:?}",
                        gen,
                        mode,
                        segment_erase_len(on_screen.as_deref(), erase_hint),
                        started.elapsed().as_millis(),
                        req.text
                    );
                    writer.set_on_screen(Some(req.text.clone())).await;
                }
                Ok(Err(e)) | Err(e) => {
                    eprintln!("[composer-write] failed: {e}");
                    if e.starts_with("focus:") {
                        notify_focus_abort(
                            "Focus left the target window — composition aborted. Reset or double-tap to restart.",
                        );
                        abort_composition_session(&writer, &runtime).await;
                        break;
                    }
                }
            }
            if writer.state.lock().await.desired.is_some() {
                continue;
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_erase_len_matches_command_chars() {
        assert_eq!(preview_erase_len(None), 0);
        assert_eq!(preview_erase_len(Some("/help")), 5);
        assert_eq!(preview_erase_len(Some("/review")), 7);
        assert_eq!(preview_erase_len(Some("/plan")), 5);
    }

    #[test]
    fn fresh_pick_ignores_stale_watchdog() {
        // After a consumed prompt, on_screen can lag — hint 0 must not erase.
        assert_eq!(segment_erase_len(Some("/plan"), 0), 0);
        assert_eq!(segment_erase_len(Some("/help"), 5), 5);
        assert_eq!(segment_erase_len(None, 15), 15);
    }

    #[test]
    fn full_rewrite_only_when_unlocked() {
        assert!(uses_full_rewrite(""));
        assert!(!uses_full_rewrite("/help "));
    }

    #[test]
    fn compose_stacks_locked_and_preview() {
        assert_eq!(compose_field("", "/help"), "/help");
        assert_eq!(compose_field("/help ", "/review"), "/help /review");
    }

    #[tokio::test]
    async fn request_coalesces_to_latest() {
        let w = FieldWriter::new();
        let _g1 = w.request("/a".into(), 0).await;
        let g2 = w.request("/b".into(), 2).await;
        let got = w.take_desired().await.unwrap();
        assert_eq!(got.gen, g2);
        assert_eq!(got.text, "/b");
    }

    #[tokio::test]
    async fn space_commit_moves_preview_into_locked() {
        let w = FieldWriter::new();
        w.set_on_screen(Some("/help".into())).await;
        w.clear_preview().await;
        let (locked, on_screen) = w.snapshot().await;
        assert_eq!(locked, "/help ");
        assert!(on_screen.is_none());
        assert_eq!(preview_erase_len(on_screen.as_deref()), 0);
    }

    #[tokio::test]
    async fn reset_clears_locked() {
        let w = FieldWriter::new();
        w.set_on_screen(Some("/x".into())).await;
        w.clear_preview().await;
        w.reset().await;
        let (locked, on_screen) = w.snapshot().await;
        assert!(locked.is_empty());
        assert!(on_screen.is_none());
        assert!(w.state.lock().await.target_window.is_none());
    }

    #[test]
    fn focus_mismatch_error_is_prefixed() {
        let err = verify_focus("kwin:99999999").unwrap_err();
        assert!(err.starts_with("focus:"), "{err}");
    }

    #[tokio::test]
    async fn abort_clears_writer_and_composer_runtime() {
        use crate::composer::ComposerRuntime;
        use std::time::Instant;

        let w = FieldWriter::new();
        w.set_on_screen(Some("/review".into())).await;
        {
            let mut g = w.state.lock().await;
            g.target_window = Some("kwin:1".into());
            g.locked = "/help ".into();
        }
        let runtime = Arc::new(Mutex::new(ComposerRuntime::default()));
        {
            let mut rt = runtime.lock().await;
            rt.picking.insert("ai".into(), true);
            rt.index.insert("ai".into(), 1);
            rt.last_text.insert("ai".into(), "/review".into());
            rt.last_tap.insert("ai".into(), Instant::now());
            rt.committed.insert("ai".into(), 1);
        }

        abort_composition_session(&w, &runtime).await;

        let (locked, on_screen) = w.snapshot().await;
        assert!(locked.is_empty());
        assert!(on_screen.is_none());
        assert!(w.state.lock().await.target_window.is_none());
        let rt = runtime.lock().await;
        assert!(!rt.picking.get("ai").copied().unwrap_or(false));
        assert!(rt.index.is_empty());
        assert!(rt.last_text.is_empty());
        assert!(rt.last_tap.is_empty());
        assert!(rt.committed.is_empty());
    }
}
