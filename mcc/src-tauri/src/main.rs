//! HK Operator MCC — Tauri desktop shell (Linux / BlueZ).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod git_sync;

use cyberdeck_ble::{
    CyberdeckPad, HotkeySlot, MacroEvent, PadSlots, PadStatus, MODE_HID, MODE_MACRO,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

const APP_DIR: &str = "hk-operator";
const STORE_FILE: &str = "store.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub value: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub last_used: Option<String>,
    pub created_at: String,
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

fn default_separator() -> String {
    " ".into()
}
fn default_timeout_ms() -> u64 {
    4000
}
fn default_reset_on() -> Vec<String> {
    vec!["timeout".into(), "explicitClear".into()]
}

impl Default for ComposerConfig {
    fn default() -> Self {
        Self {
            // Public portfolio defaults — no private board slash names.
            commands: vec![
                "/help".into(),
                "/review".into(),
                "/plan".into(),
            ],
            separator: default_separator(),
            timeout_ms: default_timeout_ms(),
            reset_on: default_reset_on(),
        }
    }
}

fn default_composers() -> HashMap<String, ComposerConfig> {
    let mut m = HashMap::new();
    m.insert("ai".into(), ComposerConfig::default());
    m
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub actions: Vec<Action>,
    /// Keys like "0-2" → action id
    pub pad_bindings: HashMap<String, String>,
    /// Command action ids the user has approved to execute
    pub allowed_commands: HashSet<String>,
    /// Operator-facing names for presets 1–6 (MCC-only; pad still uses LED index).
    #[serde(default)]
    pub pad_preset_names: Vec<String>,
    /// Slash-command composers keyed by id (action type `composer` value = id).
    #[serde(default = "default_composers")]
    pub composers: HashMap<String, ComposerConfig>,
}

impl Store {
    fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let mut store: Self = serde_json::from_str(&s).unwrap_or_default();
                if store.composers.is_empty() {
                    store.composers = default_composers();
                }
                store
            }
            Err(_) => {
                let mut store = Self::default();
                store.composers = default_composers();
                store
            }
        }
    }

    fn save(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

struct ComposerRuntime {
    /// Current preview index while picking
    index: HashMap<String, usize>,
    /// Last press time
    last_press: HashMap<String, std::time::Instant>,
    /// Bump to cancel a pending "pause = lock in" timer
    generation: HashMap<String, u64>,
    /// True while rotating before a pause lock-in
    picking: HashMap<String, bool>,
    /// How many commands locked into this compose session (for separator)
    committed: HashMap<String, usize>,
}

impl Default for ComposerRuntime {
    fn default() -> Self {
        Self {
            index: HashMap::new(),
            last_press: HashMap::new(),
            generation: HashMap::new(),
            picking: HashMap::new(),
            committed: HashMap::new(),
        }
    }
}

struct AppState {
    store_path: PathBuf,
    store: Mutex<Store>,
    listen_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    composer: Mutex<ComposerRuntime>,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(STORE_FILE)
}

async fn with_pad<F, T>(address: Option<String>, f: F) -> Result<T, String>
where
    F: FnOnce(CyberdeckPad) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>,
{
    let (_session, adapter) = CyberdeckPad::session_adapter()
        .await
        .map_err(|e| e.to_string())?;
    let pad = if let Some(addr) = address {
        CyberdeckPad::find_by_address(&adapter, &addr)
            .await
            .map_err(|e| e.to_string())?
    } else {
        CyberdeckPad::find(&adapter)
            .await
            .map_err(|e| e.to_string())?
    };
    pad.ensure_connected().await.map_err(|e| e.to_string())?;
    f(pad).await
}

#[tauri::command]
async fn get_store(state: State<'_, Arc<AppState>>) -> Result<Store, String> {
    Ok(state.store.lock().await.clone())
}

#[tauri::command]
async fn save_store(state: State<'_, Arc<AppState>>, store: Store) -> Result<(), String> {
    let mut g = state.store.lock().await;
    *g = store;
    g.save(&state.store_path)
}

#[tauri::command]
async fn pad_status(address: Option<String>) -> Result<PadStatus, String> {
    let (_session, adapter) = CyberdeckPad::session_adapter()
        .await
        .map_err(|e| e.to_string())?;
    let pad = if let Some(addr) = address {
        CyberdeckPad::find_by_address(&adapter, &addr)
            .await
            .map_err(|e| e.to_string())?
    } else {
        CyberdeckPad::find(&adapter)
            .await
            .map_err(|e| e.to_string())?
    };
    pad.status().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn pad_read_slots(address: Option<String>) -> Result<Vec<HotkeySlot>, String> {
    with_pad(address, |pad| {
        Box::pin(async move {
            pad.read_slots()
                .await
                .map(|s| s.slots)
                .map_err(|e| e.to_string())
        })
    })
    .await
}

#[tauri::command]
async fn pad_write_slots(address: Option<String>, slots: Vec<HotkeySlot>) -> Result<(), String> {
    if slots.len() != 18 {
        return Err(format!("expected 18 slots, got {}", slots.len()));
    }
    with_pad(address, |pad| {
        Box::pin(async move {
            pad.write_slots(&PadSlots { slots })
                .await
                .map_err(|e| e.to_string())
        })
    })
    .await
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MacroFiredPayload {
    preset: u8,
    action: u8,
    action_id: Option<String>,
    result: String,
}

fn ydotoold_socket() -> PathBuf {
    std::env::var_os("YDOTOOL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/.ydotool_socket"))
}

/// Ensure ydotoold is up so we can synthesize Ctrl+V on Wayland.
fn ensure_ydotoold() {
    use std::process::Stdio;
    let sock = ydotoold_socket();
    if sock.exists() {
        return;
    }
    let sock_str = sock.to_string_lossy().into_owned();
    let _ = Command::new("ydotoold")
        .args(["-p", &sock_str, "-P", "0666"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    std::thread::sleep(std::time::Duration::from_millis(250));
}

/// Paste into the focused window (Ctrl+V via uinput / ydotool).
fn auto_paste() -> Result<(), String> {
    ensure_ydotoold();
    let sock = ydotoold_socket();
    // Let Klipper/compositor publish clipboard before the paste chord.
    std::thread::sleep(std::time::Duration::from_millis(100));
    // KEY_LEFTCTRL=29, KEY_V=47
    let status = Command::new("ydotool")
        .env("YDOTOOL_SOCKET", &sock)
        .args(["key", "--key-delay=12", "29:1", "47:1", "47:0", "29:0"])
        .status()
        .map_err(|e| format!("ydotool failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("ydotool key exited {status}"));
    }
    Ok(())
}

/// Undo last edit in the focused window (Ctrl+Z via ydotool).
fn auto_undo() -> Result<(), String> {
    ensure_ydotoold();
    let sock = ydotoold_socket();
    std::thread::sleep(std::time::Duration::from_millis(40));
    // KEY_LEFTCTRL=29, KEY_Z=44
    let status = Command::new("ydotool")
        .env("YDOTOOL_SOCKET", &sock)
        .args(["key", "--key-delay=12", "29:1", "44:1", "44:0", "29:0"])
        .status()
        .map_err(|e| format!("ydotool undo failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("ydotool undo exited {status}"));
    }
    Ok(())
}

fn paste_text(text: &str, label: &str) -> Result<String, String> {
    // Prefer KDE Klipper on this host — arboard alone often writes a
    // clipboard backend with no wl-clipboard/xclip installed.
    let klipper = Command::new("qdbus6")
        .args([
            "org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.setClipboardContents",
            text,
        ])
        .status();
    let klipper_ok = matches!(klipper, Ok(s) if s.success());
    if !klipper_ok {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        clipboard
            .set_text(text)
            .map_err(|e| e.to_string())?;
    }
    let paste_msg = match auto_paste() {
        Ok(()) => "auto-pasted".to_string(),
        Err(e) => format!("copied (paste failed: {e})"),
    };
    let _ = Command::new("notify-send")
        .args([
            "-a",
            "MCC Pad",
            &format!("{paste_msg} · {label}"),
            "Focus the target field before pressing the pad",
        ])
        .status();
    Ok(format!("{paste_msg} {label}"))
}

async fn execute_action(
    store: &mut Store,
    runtime: &mut ComposerRuntime,
    state: Arc<AppState>,
    action: &Action,
) -> Result<String, String> {
    match action.type_.as_str() {
        "url" => {
            if !action.value.starts_with("http://") && !action.value.starts_with("https://") {
                return Err("Blocked: not an http(s) URL".into());
            }
            open::that(&action.value).map_err(|e| e.to_string())?;
            Ok(format!("opened {}", action.name))
        }
        "path" => {
            let expanded = shellexpand_home(&action.value);
            open::that(&expanded).map_err(|e| e.to_string())?;
            Ok(format!("opened path {}", action.name))
        }
        "prompt" | "note" => paste_text(&action.value, &action.name),
        "composer" => {
            // Live-rotate: rapid presses replace the preview (undo + paste next).
            // Pause ≥ timeoutMs locks the current slash into the prompt; next
            // burst picks the following command to stack.
            let id = action.value.trim();
            if id.is_empty() {
                return Err("composer action needs a composer id in value (e.g. ai)".into());
            }
            let cfg = store
                .composers
                .get(id)
                .cloned()
                .ok_or_else(|| format!("unknown composer \"{id}\""))?;
            if cfg.commands.is_empty() {
                return Err(format!("composer \"{id}\" has no commands"));
            }
            let len = cfg.commands.len();
            let timeout_ms = cfg.timeout_ms.max(500);
            let timeout = std::time::Duration::from_millis(timeout_ms);
            let now = std::time::Instant::now();
            let was_picking = runtime.picking.get(id).copied().unwrap_or(false);
            let committed = runtime.committed.get(id).copied().unwrap_or(0);

            // Very long idle → fresh compose session
            if let Some(last) = runtime.last_press.get(id) {
                if now.duration_since(*last) >= timeout.saturating_mul(3) {
                    runtime.committed.insert(id.to_string(), 0);
                    runtime.picking.insert(id.to_string(), false);
                    runtime.index.insert(id.to_string(), 0);
                }
            }
            let committed = runtime.committed.get(id).copied().unwrap_or(committed);

            let idx = if was_picking {
                let cur = runtime.index.get(id).copied().unwrap_or(0) % len;
                let next = (cur + 1) % len;
                // Replace previous preview in the focused field
                if let Err(e) = auto_undo() {
                    eprintln!("[mcc] composer undo failed: {e}");
                }
                next
            } else {
                0
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
            let my_gen = *gen;

            // After idle pause, lock in current preview and end this pick burst.
            let id_owned = id.to_string();
            let st = state.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(timeout).await;
                let mut rt = st.composer.lock().await;
                if rt.generation.get(&id_owned).copied().unwrap_or(0) == my_gen
                    && rt.picking.get(&id_owned).copied().unwrap_or(false)
                {
                    let n = rt.committed.get(&id_owned).copied().unwrap_or(0) + 1;
                    rt.committed.insert(id_owned.clone(), n);
                    rt.picking.insert(id_owned.clone(), false);
                    rt.index.insert(id_owned, 0);
                    let _ = Command::new("notify-send")
                        .args([
                            "-a",
                            "MCC Pad",
                            "composer locked",
                            "Paused — current slash kept. Press again to pick the next.",
                        ])
                        .status();
                }
            });

            paste_text(
                &text,
                &format!(
                    "{} rotate [{}] ({}/{}) — pause {}ms to lock",
                    action.name,
                    token,
                    idx + 1,
                    len,
                    timeout_ms
                ),
            )
        }
        "command" => {
            if !store.allowed_commands.contains(&action.id) {
                return Err(format!(
                    "Command \"{}\" not allowed yet — approve it in the UI first",
                    action.name
                ));
            }
            let status = Command::new("bash")
                .arg("-lc")
                .arg(&action.value)
                .spawn()
                .map_err(|e| e.to_string())?;
            let _ = status; // fire-and-forget
            Ok(format!("ran {}", action.name))
        }
        other => Err(format!("unknown action type: {other}")),
    }
}

fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

#[tauri::command]
async fn allow_command(state: State<'_, Arc<AppState>>, action_id: String) -> Result<(), String> {
    let mut g = state.store.lock().await;
    g.allowed_commands.insert(action_id);
    g.save(&state.store_path)
}

#[tauri::command]
async fn execute_action_id(
    state: State<'_, Arc<AppState>>,
    action_id: String,
) -> Result<String, String> {
    let st = state.inner().clone();
    let mut store = st.store.lock().await;
    let action = store
        .actions
        .iter()
        .find(|a| a.id == action_id)
        .cloned()
        .ok_or_else(|| "action not found".to_string())?;
    let mut runtime = st.composer.lock().await;
    let result = execute_action(&mut store, &mut runtime, st.clone(), &action).await?;
    drop(runtime);
    store.save(&st.store_path)?;
    Ok(result)
}

#[tauri::command]
async fn reset_composer(
    state: State<'_, Arc<AppState>>,
    composer_id: Option<String>,
) -> Result<(), String> {
    let mut runtime = state.composer.lock().await;
    if let Some(id) = composer_id {
        runtime.index.insert(id.clone(), 0);
        runtime.last_press.remove(&id);
        runtime.picking.insert(id.clone(), false);
        runtime.committed.insert(id.clone(), 0);
        let gen = runtime.generation.entry(id).or_insert(0);
        *gen += 1;
    } else {
        *runtime = ComposerRuntime::default();
    }
    Ok(())
}

fn profiles_dir(store_path: &PathBuf) -> PathBuf {
    store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("profiles")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileFile {
    actions: Vec<Action>,
    pad_bindings: HashMap<String, String>,
    #[serde(default)]
    pad_preset_names: Vec<String>,
    #[serde(default)]
    composers: HashMap<String, ComposerConfig>,
    #[serde(default)]
    allowed_commands: HashSet<String>,
}

#[tauri::command]
async fn export_profile(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<String, String> {
    let name = name.trim().replace(['/', '\\', '\0'], "_");
    if name.is_empty() {
        return Err("profile name required".into());
    }
    let store = state.store.lock().await;
    let dir = profiles_dir(&state.store_path);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    let profile = ProfileFile {
        actions: store.actions.clone(),
        pad_bindings: store.pad_bindings.clone(),
        pad_preset_names: store.pad_preset_names.clone(),
        composers: store.composers.clone(),
        allowed_commands: store.allowed_commands.clone(),
    };
    let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    // settings.json bookmark
    let settings_path = state
        .store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("settings.json");
    let settings = serde_json::json!({
        "activeProfile": name,
        "lastExportedAt": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    let _ = std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    );
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn import_profile(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> Result<Store, String> {
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let profile: ProfileFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut store = state.store.lock().await;
    apply_profile_file(&mut store, profile);
    store.save(&state.store_path)?;
    let out = store.clone();
    drop(store);
    // Reset composer cycle after profile load
    let mut runtime = state.composer.lock().await;
    *runtime = ComposerRuntime::default();
    let settings_path = state
        .store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("settings.json");
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let settings = serde_json::json!({
        "activeProfile": name,
        "lastImportedAt": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    let _ = std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    );
    Ok(out)
}

fn config_dir_from_store(store_path: &PathBuf) -> PathBuf {
    store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn apply_profile_file(store: &mut Store, profile: ProfileFile) {
    store.actions = profile.actions;
    store.pad_bindings = profile.pad_bindings;
    store.pad_preset_names = profile.pad_preset_names;
    store.composers = if profile.composers.is_empty() {
        default_composers()
    } else {
        profile.composers
    };
    store.allowed_commands = profile.allowed_commands;
}

#[tauri::command]
async fn git_sync_status(state: State<'_, Arc<AppState>>) -> Result<git_sync::GitSyncStatus, String> {
    let dir = config_dir_from_store(&state.store_path);
    Ok(git_sync::status(&dir))
}

#[tauri::command]
async fn git_sync_set_remote(
    state: State<'_, Arc<AppState>>,
    remote: String,
) -> Result<git_sync::GitSyncStatus, String> {
    let dir = config_dir_from_store(&state.store_path);
    git_sync::set_remote(&dir, &remote)?;
    Ok(git_sync::status(&dir))
}

#[tauri::command]
async fn git_sync_ensure_repo(
    state: State<'_, Arc<AppState>>,
) -> Result<git_sync::GitSyncStatus, String> {
    let dir = config_dir_from_store(&state.store_path);
    git_sync::ensure_local_repo(&dir)?;
    Ok(git_sync::status(&dir))
}

#[tauri::command]
async fn git_sync_pull(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let dir = config_dir_from_store(&state.store_path);
    git_sync::pull(&dir)
}

#[tauri::command]
async fn git_sync_push(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let dir = config_dir_from_store(&state.store_path);
    git_sync::push_all(&dir)
}

#[tauri::command]
async fn git_sync_push_profile(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<String, String> {
    let dir = config_dir_from_store(&state.store_path);
    let store = state.store.lock().await;
    let profile = ProfileFile {
        actions: store.actions.clone(),
        pad_bindings: store.pad_bindings.clone(),
        pad_preset_names: store.pad_preset_names.clone(),
        composers: store.composers.clone(),
        allowed_commands: store.allowed_commands.clone(),
    };
    drop(store);
    let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    let path = git_sync::write_profile_file(&dir, &name, &json)?;
    let push = git_sync::push_all(&dir)?;
    let mut settings = git_sync::load_settings(&dir);
    settings.active_profile = Some(git_sync::sanitize_profile_name(&name)?);
    git_sync::save_settings(&dir, &settings)?;
    Ok(format!("Wrote {} · {push}", path.display()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullApplyResult {
    store: Store,
    profile: String,
    action_count: usize,
    /// Git pull outcome (success text, or "pull failed: … (applied local copy)").
    pull_message: String,
    /// True when applied profile matches the previous live store (nothing visible changed).
    unchanged: bool,
}

#[tauri::command]
async fn git_sync_pull_apply(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<PullApplyResult, String> {
    let dir = config_dir_from_store(&state.store_path);
    let pull_message = match git_sync::pull(&dir) {
        Ok(m) => m,
        Err(e) => format!("pull failed: {e} (applied local copy)"),
    };
    let raw = git_sync::read_profile_file(&dir, &name)?;
    let profile: ProfileFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let action_count = profile.actions.len();
    let mut store = state.store.lock().await;
    let before = serde_json::to_string(&*store).unwrap_or_default();
    apply_profile_file(&mut store, profile);
    store.save(&state.store_path)?;
    let out = store.clone();
    let after = serde_json::to_string(&out).unwrap_or_default();
    let unchanged = before == after;
    drop(store);
    let mut runtime = state.composer.lock().await;
    *runtime = ComposerRuntime::default();
    let mut settings = git_sync::load_settings(&dir);
    let clean_name = git_sync::sanitize_profile_name(&name)?;
    settings.active_profile.replace(clean_name.clone());
    settings.last_pull_at = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let _ = git_sync::save_settings(&dir, &settings);
    Ok(PullApplyResult {
        store: out,
        profile: clean_name,
        action_count,
        pull_message,
        unchanged,
    })
}

#[tauri::command]
async fn gh_auth_status() -> Result<git_sync::GhAuthStatus, String> {
    Ok(git_sync::gh_auth_status())
}

#[tauri::command]
async fn gh_auth_login() -> Result<String, String> {
    git_sync::open_gh_login()
}

#[tauri::command]
async fn git_sync_create_repo(
    state: State<'_, Arc<AppState>>,
    name: String,
    private_repo: bool,
) -> Result<String, String> {
    let dir = config_dir_from_store(&state.store_path);
    git_sync::create_github_repo(&dir, &name, private_repo)
}

#[tauri::command]
async fn start_macro_listen(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    address: Option<String>,
) -> Result<(), String> {
    start_macro_listen_inner(app, state.inner().clone(), address).await
}

#[tauri::command]
async fn stop_macro_listen(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut stop = state.listen_stop.lock().await;
    if let Some(tx) = stop.take() {
        let _ = tx.send(());
    }
    Ok(())
}

#[tauri::command]
fn mode_constants() -> serde_json::Value {
    serde_json::json!({ "hid": MODE_HID, "macro": MODE_MACRO })
}

/// Fire a pad binding by key ("2-0") — used by KDE F13–F15 shortcuts / localhost API.
async fn fire_binding_key(state: &Arc<AppState>, app: &AppHandle, key: &str) -> String {
    let mut store = state.store.lock().await;
    let action_id = store.pad_bindings.get(key).cloned();
    let result = if let Some(ref id) = action_id {
        if let Some(act) = store.actions.iter().find(|a| &a.id == id).cloned() {
            let mut runtime = state.composer.lock().await;
            match execute_action(&mut store, &mut runtime, state.clone(), &act).await {
                Ok(msg) => {
                    drop(runtime);
                    let _ = store.save(&state.store_path);
                    msg
                }
                Err(e) => e,
            }
        } else {
            format!("binding {key} points to missing action")
        }
    } else {
        format!("no binding for slot {key}")
    };
    let parts: Vec<&str> = key.split('-').collect();
    let preset = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let action = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let payload = MacroFiredPayload {
        preset,
        action,
        action_id,
        result: result.clone(),
    };
    let _ = app.emit("macro-fired", payload);
    result
}

#[tauri::command]
async fn fire_binding(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    key: String,
) -> Result<String, String> {
    Ok(fire_binding_key(state.inner(), &app, &key).await)
}

fn spawn_localhost_fire_api(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = match TcpListener::bind("127.0.0.1:17321").await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[mcc] localhost fire API bind failed: {e}");
                return;
            }
        };
        eprintln!("[mcc] localhost fire API on http://127.0.0.1:17321/fire/{{p}}-{{a}}");
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let mut buf = vec![0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let mut key = None;
            for line in req.lines() {
                if let Some(rest) = line.strip_prefix("POST /fire/") {
                    let path = rest.split_whitespace().next().unwrap_or("");
                    key = Some(path.trim_matches('/').to_string());
                    break;
                }
                if let Some(rest) = line.strip_prefix("GET /fire/") {
                    let path = rest.split_whitespace().next().unwrap_or("");
                    key = Some(path.trim_matches('/').to_string());
                    break;
                }
            }
            let body = if let Some(k) = key {
                fire_binding_key(&state, &app, &k).await
            } else {
                "usage: POST /fire/2-0".into()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
}

/// Install/refresh KDE global shortcuts that fire MCC bindings.
///
/// Plasma often ignores F13+ from BLE keyboards, so we piggyback on the user's
/// existing Ctrl+Alt+1/2/3 service shortcuts (net.local.open-*) by pointing
/// their Exec at the localhost fire API. Pad preset 3 should be HID Ctrl+Alt+N.
fn install_kde_fire_shortcuts() {
    let apps = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("applications");
    let _ = std::fs::create_dir_all(&apps);
    // Dedicated F-key entries (harmless if Plasma ignores them from BLE).
    let fkeys = [
        ("0", "F13", "2-0"),
        ("1", "F14", "2-1"),
        ("2", "F15", "2-2"),
    ];
    for (i, fkey, slot) in fkeys {
        let path = apps.join(format!("net.local.mcc-macro-{i}.desktop"));
        let body = format!(
            "[Desktop Entry]\n\
Type=Application\n\
Name=MCC Pad Macro {slot}\n\
NoDisplay=true\n\
StartupNotify=false\n\
Exec=curl -s -X POST http://127.0.0.1:17321/fire/{slot}\n\
X-KDE-GlobalAccel-CommandShortcut=true\n\
X-KDE-Shortcuts={fkey}\n"
        );
        let _ = std::fs::write(&path, body);
    }
    // Reliable path: reuse Ctrl+Alt+1/2/3 (already bound in kglobalaccel).
    let openers = [
        ("net.local.open-task-app.desktop", "Ctrl+Alt+1", "2-0", "MCC fire 2-0"),
        ("net.local.open-sysmon.desktop", "Ctrl+Alt+2", "2-1", "MCC fire 2-1"),
        ("net.local.open-vscode.desktop", "Ctrl+Alt+3", "2-2", "MCC fire 2-2"),
    ];
    for (file, chord, slot, name) in openers {
        let path = apps.join(file);
        let body = format!(
            "[Desktop Entry]\n\
Type=Application\n\
Name={name}\n\
NoDisplay=true\n\
StartupNotify=false\n\
Exec=curl -s -X POST http://127.0.0.1:17321/fire/{slot}\n\
X-KDE-GlobalAccel-CommandShortcut=true\n\
X-KDE-Shortcuts={chord}\n"
        );
        let _ = std::fs::write(&path, body);
    }
    let _ = Command::new("kbuildsycoca6").arg("--noincremental").output();
    let _ = Command::new("kbuildsycoca5").arg("--noincremental").output();
}

fn main() {
    let store_path = config_path();
    let store = Store::load(&store_path);
    let state = Arc::new(AppState {
        store_path,
        store: Mutex::new(store),
        listen_stop: Mutex::new(None),
        composer: Mutex::new(ComposerRuntime::default()),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            spawn_localhost_fire_api(handle.clone(), state.clone());
            install_kde_fire_shortcuts();
            ensure_ydotoold();
            // Also keep BLE notify listen as secondary (works after firmware fix).
            let st = state.clone();
            let h2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = start_macro_listen_inner(h2, st, None).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_store,
            save_store,
            pad_status,
            pad_read_slots,
            pad_write_slots,
            allow_command,
            execute_action_id,
            reset_composer,
            export_profile,
            import_profile,
            git_sync_status,
            git_sync_set_remote,
            git_sync_ensure_repo,
            git_sync_pull,
            git_sync_push,
            git_sync_push_profile,
            git_sync_pull_apply,
            gh_auth_status,
            gh_auth_login,
            git_sync_create_repo,
            start_macro_listen,
            stop_macro_listen,
            mode_constants,
            fire_binding,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn start_macro_listen_inner(
    app: AppHandle,
    state: Arc<AppState>,
    address: Option<String>,
) -> Result<(), String> {
    {
        let mut stop = state.listen_stop.lock().await;
        if let Some(tx) = stop.take() {
            let _ = tx.send(());
        }
    }
    let (tx, mut rx_stop) = tokio::sync::oneshot::channel::<()>();
    *state.listen_stop.lock().await = Some(tx);

    let app_state = state.clone();
    let app2 = app.clone();

    tauri::async_runtime::spawn(async move {
        let pad = match with_pad(address, |pad| Box::pin(async move { Ok(pad) })).await {
            Ok(p) => p,
            Err(e) => {
                let _ = app2.emit("pad-error", e);
                return;
            }
        };

        let mut events = match pad.subscribe_macro_events().await {
            Ok(rx) => rx,
            Err(e) => {
                let _ = app2.emit("pad-error", e.to_string());
                return;
            }
        };

        let _ = app2.emit("pad-listening", true);

        loop {
            tokio::select! {
                _ = &mut rx_stop => break,
                ev = events.recv() => {
                    let Some(MacroEvent { preset, action }) = ev else { break };
                    let key = PadSlots::binding_key(preset as usize, action as usize);
                    let result = fire_binding_key(&app_state, &app2, &key).await;
                    let _ = result;
                }
            }
        }

        let _ = app2.emit("pad-listening", false);
    });

    Ok(())
}
