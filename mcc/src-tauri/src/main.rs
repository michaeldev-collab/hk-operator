//! 3DL Macro Command Center — Tauri desktop shell (Linux / BlueZ).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod composer;
mod composer_write;
mod git_sync;
mod space_listen;
mod ydotool_sock;

use composer::{
    commit_composer, default_double_tap_ms, note_composer_tap, reset_composer_runtime,
    ComposerConfig, ComposerRuntime, TapOutcome,
};
use composer_write::FieldWriter;
use cyberdeck_ble::{
    CyberdeckPad, HotkeySlot, MacroEvent, PadSlots, PadStatus, ACTION_COUNT, BANK_COUNT, MODE_HID,
    MODE_MACRO, PRESET_COUNT, SLOT_COUNT,
};
use cyberdeck_dongle::{DongleError, DonglePad};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{Mutex, Notify, OwnedMutexGuard};
use ydotool_sock::ensure_ydotoold;

const APP_DIR: &str = "3dl-macro-command-center";
const STORE_FILE: &str = "store.json";
const STORE_SCHEMA_VERSION: u8 = 3;
const MAX_STARTUP_MACRO_DRAIN: usize = 64;
static DONGLE_IO: StdMutex<Option<DonglePad>> = StdMutex::new(None);
/// Serializes every operation that can select a bank, transfer a slots page,
/// or change which transport owns the pad. `DONGLE_IO` frames individual CDC
/// commands; this lock protects the larger select/read/write/restore transaction.
static PAD_IO: Mutex<()> = Mutex::const_new(());

/// Reuse one CDC/libusb transport across status, sync, and macro polling.
/// Dropping and reopening the device on every 150 ms poll can churn the kernel
/// CDC driver; a transport error clears the cached session so the next command
/// can reconnect cleanly after a device reset or unplug.
fn with_dongle_session<T>(
    operation: impl FnOnce(&mut DonglePad) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let mut session = DONGLE_IO
        .lock()
        .map_err(|_| "dongle I/O lock poisoned".to_string())?;
    if session.is_none() {
        match DonglePad::open() {
            Ok(dongle) => *session = Some(dongle),
            Err(DongleError::NotFound) => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "S3 dongle is present but unavailable; refusing BlueZ fallback: {error}"
                ))
            }
        }
    }
    let result = operation(session.as_mut().expect("dongle session initialized"));
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            *session = None;
            Err(error)
        }
    }
}

/// Empty the S3's firmware-side MacroEvent queue before a listener epoch is
/// allowed to dispatch. The queue is 16 entries today; the larger bound
/// tolerates events arriving during startup while still failing closed under
/// a stuck or continuously refilled queue.
fn drain_macro_queue_with(
    mut poll_has_event: impl FnMut() -> Result<bool, String>,
) -> Result<usize, String> {
    for drained in 0..MAX_STARTUP_MACRO_DRAIN {
        if !poll_has_event()? {
            return Ok(drained);
        }
    }
    Err(format!(
        "S3 macro queue did not drain after {MAX_STARTUP_MACRO_DRAIN} events; listener stopped"
    ))
}

fn drain_dongle_macro_queue(dongle: &mut DonglePad) -> Result<usize, String> {
    drain_macro_queue_with(|| {
        dongle
            .poll_events()
            .map(|poll| poll.macro_event.is_some())
            .map_err(|error| error.to_string())
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

fn default_composers() -> HashMap<String, ComposerConfig> {
    let mut m = HashMap::new();
    m.insert("ai".into(), ComposerConfig::default());
    m
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    pub schema_version: u8,
    pub actions: Vec<Action>,
    /// Keys like "bank-preset-action" → action id.
    #[serde(default)]
    pub pad_bindings: HashMap<String, String>,
    /// Command action ids the user has approved to execute
    #[serde(default)]
    pub allowed_commands: HashSet<String>,
    /// Durable binding between an approval and the exact shell text reviewed by
    /// the operator. The frontend intentionally does not grant through this map;
    /// only `allow_command` can create an entry.
    #[serde(default)]
    pub approved_command_values: HashMap<String, String>,
    /// Operator-facing names for presets 1–6 (MCC-only; pad still uses LED index).
    #[serde(default)]
    pub pad_preset_names: Vec<String>,
    /// Slash-command composers keyed by id (action type `composer` value = id).
    #[serde(default = "default_composers")]
    pub composers: HashMap<String, ComposerConfig>,
    /// Five banks of 18 pad HID/macro slots. Legacy flat arrays migrate to bank 0.
    #[serde(default, deserialize_with = "deserialize_pad_slots")]
    pub pad_slots: Option<Vec<Vec<HotkeySlot>>>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema_version: STORE_SCHEMA_VERSION,
            actions: Vec::new(),
            pad_bindings: HashMap::new(),
            allowed_commands: HashSet::new(),
            approved_command_values: HashMap::new(),
            pad_preset_names: Vec::new(),
            composers: default_composers(),
            pad_slots: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum PadSlotsWire {
    Flat(Vec<HotkeySlot>),
    Banks(Vec<Vec<HotkeySlot>>),
}

fn empty_slot_bank() -> Vec<HotkeySlot> {
    (0..SLOT_COUNT)
        .map(|_| HotkeySlot {
            mode: MODE_HID,
            r#mod: 0,
            key: 0,
            label: String::new(),
        })
        .collect()
}

fn deserialize_pad_slots<'de, D>(deserializer: D) -> Result<Option<Vec<Vec<HotkeySlot>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let wire = Option::<PadSlotsWire>::deserialize(deserializer)?;
    match wire {
        None => Ok(None),
        Some(PadSlotsWire::Flat(slots)) if slots.len() == SLOT_COUNT => {
            let mut banks = vec![empty_slot_bank(); BANK_COUNT];
            banks[0] = slots;
            Ok(Some(banks))
        }
        Some(PadSlotsWire::Banks(banks))
            if banks.len() == BANK_COUNT && banks.iter().all(|b| b.len() == SLOT_COUNT) =>
        {
            Ok(Some(banks))
        }
        Some(PadSlotsWire::Flat(slots)) => Err(D::Error::custom(format!(
            "legacy padSlots has {} entries, expected {SLOT_COUNT}",
            slots.len()
        ))),
        Some(PadSlotsWire::Banks(banks)) => Err(D::Error::custom(format!(
            "banked padSlots must be {BANK_COUNT} banks of {SLOT_COUNT} entries (got {} banks)",
            banks.len()
        ))),
    }
}

fn parse_binding_key(key: &str) -> Option<(usize, usize, usize, bool)> {
    let parts: Vec<_> = key.split('-').collect();
    let (bank, preset, action, legacy) = match parts.as_slice() {
        [preset, action] => (0, preset.parse().ok()?, action.parse().ok()?, true),
        [bank, preset, action] => (
            bank.parse().ok()?,
            preset.parse().ok()?,
            action.parse().ok()?,
            false,
        ),
        _ => return None,
    };
    if bank >= BANK_COUNT || preset >= PRESET_COUNT || action >= ACTION_COUNT {
        return None;
    }
    Some((bank, preset, action, legacy))
}

fn migrate_pad_bindings(bindings: &HashMap<String, String>) -> HashMap<String, String> {
    let mut migrated = HashMap::new();
    for (key, value) in bindings {
        if parse_binding_key(key).is_some_and(|(_, _, _, legacy)| !legacy) {
            migrated.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in bindings {
        match parse_binding_key(key) {
            Some((bank, preset, action, true)) => {
                migrated
                    .entry(PadSlots::bank_binding_key(bank, preset, action))
                    .or_insert_with(|| value.clone());
            }
            Some((_, _, _, false)) => {}
            None => {
                // Preserve unknown keys for manual recovery, but validated
                // dispatch will never execute them.
                migrated.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    migrated
}

fn validate_bank(bank: u8) -> Result<(), String> {
    if bank as usize >= BANK_COUNT {
        return Err(format!("bank {bank} outside 0..{}", BANK_COUNT - 1));
    }
    Ok(())
}

impl Store {
    fn normalize(&mut self) -> Result<(), String> {
        if self.schema_version > STORE_SCHEMA_VERSION {
            return Err(format!(
                "store schema {} is newer than supported schema {STORE_SCHEMA_VERSION}",
                self.schema_version
            ));
        }

        validate_action_ids(&self.actions)?;

        // v0.2 keys were "preset-action". Explicit v0.3 bank-0 keys win
        // if both forms are present.
        self.pad_bindings = migrate_pad_bindings(&self.pad_bindings);
        self.schema_version = STORE_SCHEMA_VERSION;
        if self.composers.is_empty() {
            self.composers = default_composers();
        }
        self.reconcile_command_approvals();
        Ok(())
    }

    fn command_action(&self, id: &str) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| action.id == id && action.type_ == "command")
    }

    fn command_is_approved(&self, action: &Action) -> bool {
        action.type_ == "command"
            && self.allowed_commands.contains(&action.id)
            && self
                .approved_command_values
                .get(&action.id)
                .is_some_and(|value| value == &action.value)
    }

    fn reconcile_command_approvals(&mut self) {
        let valid: HashMap<String, String> = self
            .actions
            .iter()
            .filter(|action| action.type_ == "command")
            .map(|action| (action.id.clone(), action.value.clone()))
            .collect();
        self.allowed_commands.retain(|id| {
            valid.get(id).is_some_and(|value| {
                self.approved_command_values
                    .get(id)
                    .is_some_and(|approved| approved == value)
            })
        });
        self.approved_command_values
            .retain(|id, value| self.allowed_commands.contains(id) && valid.get(id) == Some(value));
    }

    fn load(path: &PathBuf) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let mut store: Self = serde_json::from_str(&s).map_err(|e| {
                    format!(
                        "refusing to replace unreadable store {}: {e}",
                        path.display()
                    )
                })?;
                store.normalize()?;
                Ok(store)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("failed to read store {}: {e}", path.display())),
        }
    }

    fn save(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| format!("open {}: {e}", tmp.display()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
    }
}

fn validate_action_ids(actions: &[Action]) -> Result<(), String> {
    let mut seen = HashSet::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        if action.id.trim().is_empty() {
            return Err(format!("action at index {index} has an empty id"));
        }
        if !seen.insert(action.id.as_str()) {
            return Err(format!("duplicate action id {:?}", action.id));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ActionDispatchState {
    accepting: bool,
    in_flight: usize,
    epoch: u64,
}

/// Closes action dispatch while a profile replaces both the local store and
/// pad state. Dispatch uses a synchronous try-enter operation deliberately:
/// events received while the gate is closed are dropped instead of waiting to
/// execute against the newly installed profile.
struct ActionDispatchGate {
    state: StdMutex<ActionDispatchState>,
    idle: Notify,
    replacement: Arc<Mutex<()>>,
}

impl Default for ActionDispatchGate {
    fn default() -> Self {
        Self {
            state: StdMutex::new(ActionDispatchState {
                accepting: true,
                in_flight: 0,
                epoch: 1,
            }),
            idle: Notify::new(),
            replacement: Arc::new(Mutex::new(())),
        }
    }
}

impl ActionDispatchGate {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, ActionDispatchState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn try_enter(self: &Arc<Self>) -> Option<ActionDispatchPermit> {
        self.try_enter_epoch(None)
    }

    /// Listener events carry the epoch captured when their transport began.
    /// A pre-replacement event cannot become valid merely because it reaches
    /// dispatch after the replacement gate has reopened.
    fn try_enter_epoch(self: &Arc<Self>, event_epoch: Option<u64>) -> Option<ActionDispatchPermit> {
        let mut state = self.lock_state();
        if !state.accepting || event_epoch.is_some_and(|epoch| epoch != state.epoch) {
            return None;
        }
        state.in_flight += 1;
        drop(state);
        Some(ActionDispatchPermit { gate: self.clone() })
    }

    async fn begin_replacement(self: &Arc<Self>) -> ActionReplacementGuard {
        // Only one replacement may close/reopen the shared gate at a time.
        let serial = self.replacement.clone().lock_owned().await;
        {
            let mut state = self.lock_state();
            state.accepting = false;
            state.epoch = state.epoch.wrapping_add(1).max(1);
        }

        loop {
            // `notify_one` retains a permit if the final action exits between
            // this check and the await, avoiding a lost wake-up.
            let idle = self.idle.notified();
            if self.lock_state().in_flight == 0 {
                break;
            }
            idle.await;
        }

        ActionReplacementGuard {
            gate: self.clone(),
            _serial: serial,
        }
    }

    #[cfg(test)]
    fn is_accepting(&self) -> bool {
        self.lock_state().accepting
    }

    fn current_epoch(&self) -> u64 {
        self.lock_state().epoch
    }
}

struct ActionDispatchPermit {
    gate: Arc<ActionDispatchGate>,
}

impl Drop for ActionDispatchPermit {
    fn drop(&mut self) {
        let became_idle = {
            let mut state = self.gate.lock_state();
            debug_assert!(state.in_flight > 0);
            state.in_flight = state.in_flight.saturating_sub(1);
            state.in_flight == 0
        };
        if became_idle {
            self.gate.idle.notify_one();
        }
    }
}

struct ActionReplacementGuard {
    gate: Arc<ActionDispatchGate>,
    _serial: OwnedMutexGuard<()>,
}

impl Drop for ActionReplacementGuard {
    fn drop(&mut self) {
        self.gate.lock_state().accepting = true;
    }
}

#[derive(Default)]
struct ListenerControl {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    route: Option<ListenerRoute>,
}

#[derive(Clone)]
struct ListenerRoute {
    address: Option<String>,
}

struct AppState {
    store_path: PathBuf,
    store: Mutex<Store>,
    listen_control: Mutex<ListenerControl>,
    /// Serializes stop/join/start so two callers cannot install listeners at once.
    listen_lifecycle: Mutex<()>,
    listen_generation: AtomicU64,
    /// Drops newly received actions while profile/Git replacement drains
    /// already-running actions and updates the store plus pad as one scope.
    action_dispatch: Arc<ActionDispatchGate>,
    composer: Arc<Mutex<ComposerRuntime>>,
    /// Coalesced select-all + paste worker (latest desired wins).
    field_writer: Arc<FieldWriter>,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(STORE_FILE)
}

async fn with_pad<F, T>(address: Option<String>, f: F) -> Result<T, String>
where
    F: FnOnce(
        CyberdeckPad,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>,
{
    let (_session, adapter) = CyberdeckPad::session_adapter()
        .await
        .map_err(|e| e.to_string())?;
    // "via-s3-dongle" is a sentinel, not a BLE address; looking it up over BlueZ
    // fails and takes the macro listener down with it after a dongle session.
    let pad = if let Some(addr) = address.filter(|a| !a.is_empty() && a != "via-s3-dongle") {
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
    let mut store = store;
    let requested_allowed = store.allowed_commands.clone();
    store.normalize()?;
    // Treat approval metadata supplied through the whole-store API as
    // untrusted. Rebuild it exclusively from grants already held by `g`.
    store.allowed_commands.clear();
    store.approved_command_values.clear();
    // The allow command is the only grant path. A whole-store save may retain
    // approval only when the previously approved command text is unchanged.
    for id in requested_allowed {
        let Some(old) = g.command_action(&id) else {
            continue;
        };
        let Some(new_value) = store.command_action(&id).map(|action| action.value.clone()) else {
            continue;
        };
        if g.command_is_approved(old) && old.value == new_value {
            store.allowed_commands.insert(id.clone());
            store.approved_command_values.insert(id, new_value);
        }
    }
    store.save(&state.store_path)?;
    *g = store;
    Ok(())
}

#[tauri::command]
async fn pad_status(address: Option<String>) -> Result<PadStatus, String> {
    // Prefer S3 dongle CDC proxy when the BLE bridge is up, but still report
    // BlueZ Blocked so the UI toggle stays accurate while the dongle owns the link.
    let bluez = bluez_pad_snapshot(address.clone()).await.ok();

    let dongle_status = tokio::task::spawn_blocking(|| {
        with_dongle_session(|dongle| dongle.status().map_err(|e| e.to_string()))
    })
    .await
    .map_err(|e| format!("dongle status worker: {e}"))??;
    if let Some(mut st) = dongle_status {
        if st.connected {
            if let Some(bz) = bluez {
                st.address = bz.address;
                st.bluez_blocked = bz.bluez_blocked;
                if st.name.is_none() {
                    st.name = bz.name;
                }
            }
            return Ok(st);
        }
    }

    if let Some(st) = bluez {
        if !st.connected && st.bluez_blocked != Some(true) {
            return with_pad(address, |pad| {
                Box::pin(async move { pad.status().await.map_err(|e| e.to_string()) })
            })
            .await;
        }
        return Ok(st);
    }

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

async fn bluez_pad_snapshot(address: Option<String>) -> Result<PadStatus, String> {
    let (_session, adapter) = CyberdeckPad::session_adapter()
        .await
        .map_err(|e| e.to_string())?;
    let pad = if let Some(addr) = address.filter(|a| !a.is_empty() && a != "via-s3-dongle") {
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

/// Enable/disable BlueZ for the pad. Off = disconnect + block (dongle-friendly).
#[tauri::command]
async fn pad_set_bluez_enabled(
    enabled: bool,
    address: Option<String>,
) -> Result<PadStatus, String> {
    let _pad_io = PAD_IO.lock().await;
    let (_session, adapter) = CyberdeckPad::session_adapter()
        .await
        .map_err(|e| e.to_string())?;
    let pad = if let Some(addr) = address.filter(|a| !a.is_empty() && a != "via-s3-dongle") {
        CyberdeckPad::find_by_address(&adapter, &addr)
            .await
            .map_err(|e| e.to_string())?
    } else {
        CyberdeckPad::find(&adapter)
            .await
            .map_err(|e| e.to_string())?
    };
    pad.set_bluez_enabled(enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn pad_read_slots(address: Option<String>, bank: u8) -> Result<Vec<HotkeySlot>, String> {
    let _pad_io = PAD_IO.lock().await;
    pad_read_slots_unlocked(address, bank).await
}

async fn pad_read_slots_unlocked(
    address: Option<String>,
    bank: u8,
) -> Result<Vec<HotkeySlot>, String> {
    validate_bank(bank)?;
    let dongle_slots = tokio::task::spawn_blocking(move || {
        with_dongle_session(|d| {
            let status = d.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok(None);
            }
            if status.protocol_compatible != Some(true) {
                return Err(
                    "S3 dongle owns the pad but is not protocol v0.3; refusing BlueZ fallback"
                        .into(),
                );
            }
            if status.slots_ready != Some(true) {
                return Err("S3 dongle owns the pad but its v0.3 slots proxy is not ready".into());
            }
            d.read_slots(bank)
                .map(|s| Some(s.slots))
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .flatten();
    if let Some(slots) = dongle_slots {
        return Ok(slots);
    }
    with_pad(address, |pad| {
        Box::pin(async move {
            if bank == 0 {
                pad.read_slots().await
            } else {
                pad.read_slots_for_bank(bank).await
            }
            .map(|s| s.slots)
            .map_err(|e| e.to_string())
        })
    })
    .await
}

#[tauri::command]
async fn pad_write_slots(
    address: Option<String>,
    bank: u8,
    slots: Vec<HotkeySlot>,
) -> Result<(), String> {
    let _pad_io = PAD_IO.lock().await;
    pad_write_slots_unlocked(address, bank, slots).await
}

async fn pad_write_slots_unlocked(
    address: Option<String>,
    bank: u8,
    slots: Vec<HotkeySlot>,
) -> Result<(), String> {
    validate_bank(bank)?;
    if slots.len() != SLOT_COUNT {
        return Err(format!("expected {SLOT_COUNT} slots, got {}", slots.len()));
    }
    let dongle_slots = slots.clone();
    let wrote_dongle = tokio::task::spawn_blocking(move || {
        with_dongle_session(|d| {
            let status = d.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok(false);
            }
            if status.protocol_compatible != Some(true) {
                return Err(
                    "S3 dongle owns the pad but is not protocol v0.3; refusing BlueZ fallback"
                        .into(),
                );
            }
            if status.slots_ready != Some(true) {
                return Err("S3 dongle owns the pad but its v0.3 slots proxy is not ready".into());
            }
            d.write_slots(bank, &dongle_slots)
                .map_err(|e| e.to_string())?;
            Ok(true)
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .unwrap_or(false);
    if wrote_dongle {
        return Ok(());
    }
    with_pad(address, |pad| {
        Box::pin(async move {
            let page = PadSlots { slots };
            if bank == 0 {
                pad.write_slots(&page).await
            } else {
                pad.write_slots_for_bank(bank, &page).await
            }
            .map_err(|e| e.to_string())
        })
    })
    .await
}

#[tauri::command]
async fn pad_write_banks(
    address: Option<String>,
    banks: Vec<Vec<HotkeySlot>>,
) -> Result<String, String> {
    try_write_pad_banks(address, &banks).await
}

#[tauri::command]
async fn pad_restore_bank(address: Option<String>, bank: u8) -> Result<(), String> {
    restore_pad_bank(address, bank).await
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PadBanksReadResult {
    banks: Vec<Vec<HotkeySlot>>,
    bank_count: usize,
    restored_bank: u8,
    transport: String,
}

#[tauri::command]
async fn pad_read_banks(address: Option<String>) -> Result<PadBanksReadResult, String> {
    let _pad_io = PAD_IO.lock().await;
    let (bank_count, transport, restore_bank) = pad_bank_capacity(address.clone()).await?;
    let mut banks = Vec::with_capacity(bank_count);
    let mut read_error = None;
    for bank in 0..bank_count {
        match pad_read_slots_unlocked(address.clone(), bank as u8).await {
            Ok(slots) => banks.push(slots),
            Err(error) => {
                read_error = Some(error);
                break;
            }
        }
    }

    let restore_result = if bank_count > 1 {
        restore_pad_bank_unlocked(address, restore_bank).await
    } else {
        Ok(())
    };
    if let Some(error) = read_error {
        return Err(format!(
            "pad refresh stopped after {}/{} banks: {error}; restore bank {restore_bank}: {}",
            banks.len(),
            bank_count,
            restore_result
                .map(|_| "ok".to_string())
                .unwrap_or_else(|restore_error| restore_error)
        ));
    }
    restore_result.map_err(|error| {
        format!(
            "read {}/{} banks but failed to restore bank {restore_bank}: {error}",
            banks.len(),
            bank_count
        )
    })?;

    Ok(PadBanksReadResult {
        banks,
        bank_count,
        restored_bank: restore_bank,
        transport: transport.to_string(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MacroFiredPayload {
    bank: u8,
    preset: u8,
    action: u8,
    action_id: Option<String>,
    result: String,
}

fn ydotoold_socket() -> PathBuf {
    let config_dir = config_path().parent().map(|p| p.to_path_buf());
    ensure_ydotoold(config_dir.as_deref())
}

/// Paste into the focused window (Ctrl+V via uinput / ydotool).
fn auto_paste() -> Result<(), String> {
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
        clipboard.set_text(text).map_err(|e| e.to_string())?;
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
            // Double-tap = start/rotate. Space commits. Next double-tap stacks.
            let id = action.value.trim();
            composer::composer_precheck(id, store.composers.get(id))?;
            let cfg = store
                .composers
                .get(id)
                .cloned()
                .expect("composer_precheck verified composer exists");
            let now = std::time::Instant::now();
            match note_composer_tap(id, &cfg, runtime, now, default_double_tap_ms())? {
                TapOutcome::Arming => {
                    eprintln!("[composer] arming double-tap for {id}");
                    Ok(format!("{} — tap again quickly to rotate", action.name))
                }
                TapOutcome::Fired(press) => {
                    eprintln!(
                        "[composer] double-tap idx={} token={} rotate={}",
                        press.idx, press.token, press.replaced_preview
                    );

                    let _ = Command::new("notify-send")
                        .args([
                            "-a",
                            "MCC Pad",
                            &format!("{} ({}/{})", press.token, press.idx + 1, press.len),
                            "Double-tap rotates · Space commits · P3 B4 new loop",
                        ])
                        .status();

                    {
                        let fw = state.field_writer.clone();
                        let text = press.text.clone();
                        let erase = press.erase_chars;
                        tauri::async_runtime::spawn(async move {
                            fw.request(text, erase).await;
                        });
                    }

                    Ok(format!(
                        "{} [{}] ({}/{}) — Space to commit",
                        action.name,
                        press.token,
                        press.idx + 1,
                        press.len
                    ))
                }
            }
        }
        "composer-commit" => {
            let id = action.value.trim();
            composer::composer_precheck(id, store.composers.get(id))?;
            if commit_composer(runtime, id) {
                let fw = state.field_writer.clone();
                tauri::async_runtime::spawn(async move {
                    fw.clear_preview().await;
                });
                Ok(format!(
                    "{} committed — double-tap to stack next",
                    action.name
                ))
            } else {
                Ok(format!("{} — nothing to commit", action.name))
            }
        }
        "composer-reset" => {
            // New loop: clear FSM + field watchdog (P3 B4 / Reset cycle only).
            let id = action.value.trim();
            if id.is_empty() {
                reset_composer_runtime(runtime, None);
            } else {
                composer::composer_precheck(id, store.composers.get(id))?;
                reset_composer_runtime(runtime, Some(id));
            }
            state.field_writer.reset().await;
            let _ = Command::new("notify-send")
                .args([
                    "-a",
                    "MCC Pad",
                    "composer new loop",
                    "Session cleared — next double-tap starts fresh.",
                ])
                .status();
            Ok(format!("{} — new loop ready", action.name))
        }
        "command" => {
            if !store.command_is_approved(action) {
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
    validate_action_ids(&g.actions)?;
    let action = g
        .command_action(&action_id)
        .cloned()
        .ok_or_else(|| format!("command action {action_id:?} not found"))?;
    let mut next = g.clone();
    next.allowed_commands.insert(action.id.clone());
    next.approved_command_values.insert(action.id, action.value);
    // Persist first. A failed disk write must not leave an in-memory grant.
    next.save(&state.store_path)?;
    *g = next;
    Ok(())
}

#[tauri::command]
async fn execute_action_id(
    state: State<'_, Arc<AppState>>,
    action_id: String,
) -> Result<String, String> {
    let st = state.inner().clone();
    let _dispatch = st
        .action_dispatch
        .try_enter()
        .ok_or_else(|| "action dispatch dropped: profile replacement in progress".to_string())?;
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
    reset_composer_runtime(&mut runtime, composer_id.as_deref());
    drop(runtime);
    state.field_writer.reset().await;
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
    #[serde(default)]
    schema_version: u8,
    actions: Vec<Action>,
    pad_bindings: HashMap<String, String>,
    #[serde(default)]
    pad_preset_names: Vec<String>,
    #[serde(default)]
    composers: HashMap<String, ComposerConfig>,
    #[serde(default)]
    allowed_commands: HashSet<String>,
    /// Preserve the original wire shape so a legacy flat profile can update
    /// bank 0 without erasing bank-aware data in banks 1–4.
    #[serde(default)]
    pad_slots: Option<PadSlotsWire>,
}

fn valid_pad_banks(banks: &Option<Vec<Vec<HotkeySlot>>>) -> bool {
    banks.as_ref().is_some_and(|banks| {
        banks.len() == BANK_COUNT && banks.iter().all(|slots| slots.len() == SLOT_COUNT)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfilePadWrite {
    None,
    BankZero,
    AllBanks,
}

fn validate_profile_pad_slots(slots: &Option<PadSlotsWire>) -> Result<(), String> {
    match slots {
        None => Ok(()),
        Some(PadSlotsWire::Flat(slots)) if slots.len() == SLOT_COUNT => Ok(()),
        Some(PadSlotsWire::Banks(banks))
            if banks.len() == BANK_COUNT && banks.iter().all(|bank| bank.len() == SLOT_COUNT) =>
        {
            Ok(())
        }
        Some(PadSlotsWire::Flat(slots)) => Err(format!(
            "legacy profile padSlots has {} entries, expected {SLOT_COUNT}",
            slots.len()
        )),
        Some(PadSlotsWire::Banks(banks)) => Err(format!(
            "profile padSlots must be {BANK_COUNT} banks of {SLOT_COUNT} entries (got {} banks)",
            banks.len()
        )),
    }
}

fn profile_has_all_banks(slots: &Option<PadSlotsWire>) -> bool {
    matches!(slots, Some(PadSlotsWire::Banks(banks)) if banks.len() == BANK_COUNT
        && banks.iter().all(|bank| bank.len() == SLOT_COUNT))
}

fn action_composer_id(action: &Action) -> Option<&str> {
    matches!(
        action.type_.as_str(),
        "composer" | "composer-commit" | "composer-reset"
    )
    .then(|| action.value.trim())
    .filter(|id| !id.is_empty())
}

/// Best-effort write of one stored bank to the pad (dongle preferred, else BlueZ).
async fn try_write_pad_bank(
    address: Option<String>,
    bank: u8,
    slots: &[HotkeySlot],
) -> Result<String, String> {
    validate_bank(bank)?;
    if slots.len() != SLOT_COUNT {
        return Err(format!("expected {SLOT_COUNT} slots, got {}", slots.len()));
    }
    let slots = slots.to_vec();
    let dongle_slots = slots.clone();
    let wrote_dongle = tokio::task::spawn_blocking(move || {
        with_dongle_session(|d| {
            let status = d.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok(false);
            }
            if status.protocol_compatible != Some(true) {
                return Err(
                    "S3 dongle owns the pad but is not protocol v0.3; refusing BlueZ fallback"
                        .into(),
                );
            }
            if status.slots_ready != Some(true) {
                return Err("S3 dongle owns the pad but its v0.3 slots proxy is not ready".into());
            }
            d.write_slots(bank, &dongle_slots)
                .map_err(|e| e.to_string())?;
            Ok(true)
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .unwrap_or(false);
    if wrote_dongle {
        return Ok(format!("wrote bank {bank} via S3 dongle"));
    }
    with_pad(address, |pad| {
        Box::pin(async move {
            let page = PadSlots { slots };
            if bank == 0 {
                pad.write_slots(&page).await
            } else {
                pad.write_slots_for_bank(bank, &page).await
            }
            .map_err(|e| e.to_string())?;
            Ok(format!("wrote bank {bank} via BlueZ"))
        })
    })
    .await
}

async fn try_write_pad_banks(
    address: Option<String>,
    banks: &[Vec<HotkeySlot>],
) -> Result<String, String> {
    try_write_pad_banks_scope(address, banks, ProfilePadWrite::AllBanks).await
}

async fn try_write_pad_banks_scope(
    address: Option<String>,
    banks: &[Vec<HotkeySlot>],
    scope: ProfilePadWrite,
) -> Result<String, String> {
    if banks.len() != BANK_COUNT || banks.iter().any(|slots| slots.len() != SLOT_COUNT) {
        return Err(format!("expected {BANK_COUNT} banks of {SLOT_COUNT} slots"));
    }
    if scope == ProfilePadWrite::None {
        return Err("no pad slots were supplied for write".into());
    }
    let _pad_io = PAD_IO.lock().await;
    try_write_pad_banks_unlocked(address, banks, scope).await
}

async fn try_write_pad_banks_unlocked(
    address: Option<String>,
    banks: &[Vec<HotkeySlot>],
    scope: ProfilePadWrite,
) -> Result<String, String> {
    let (device_bank_count, expected_transport, restore_bank) =
        pad_bank_capacity(address.clone()).await?;
    let write_bank_count = match scope {
        ProfilePadWrite::None => 0,
        ProfilePadWrite::BankZero => 1,
        ProfilePadWrite::AllBanks => device_bank_count,
    };
    let mut transport = None;
    let mut completed = 0;
    let mut write_error = None;
    for (bank, slots) in banks.iter().take(write_bank_count).enumerate() {
        let msg = match try_write_pad_bank(address.clone(), bank as u8, slots).await {
            Ok(msg) => msg,
            Err(e) => {
                write_error = Some(e);
                break;
            }
        };
        completed += 1;
        transport = Some(if msg.contains("dongle") {
            "S3 dongle"
        } else {
            "BlueZ"
        });
    }
    let restore_result = if device_bank_count > 1 {
        restore_pad_bank_unlocked(address, restore_bank).await
    } else {
        Ok(())
    };
    if let Some(e) = write_error {
        return Err(format!(
            "pad sync stopped after {completed}/{write_bank_count} banks: {e}; restore bank {restore_bank}: {}",
            restore_result.map(|_| "ok".to_string()).unwrap_or_else(|e| e)
        ));
    }
    restore_result.map_err(|e| {
        format!("wrote {completed}/{write_bank_count} banks but failed to restore bank {restore_bank}: {e}")
    })?;
    Ok(format!(
        "wrote {completed} bank{} / {} slots via {}",
        if completed == 1 { "" } else { "s" },
        completed * SLOT_COUNT,
        transport.unwrap_or(expected_transport)
    ))
}

async fn pad_bank_capacity(address: Option<String>) -> Result<(usize, &'static str, u8), String> {
    let dongle = tokio::task::spawn_blocking(|| {
        with_dongle_session(|dongle| {
            let status = dongle.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok(None);
            }
            if status.protocol_compatible != Some(true) {
                return Err(
                    "S3 dongle owns the pad but is not protocol v0.3; zero banks written".into(),
                );
            }
            if status.slots_ready != Some(true) {
                return Err(
                    "S3 dongle owns the pad but its slots proxy is not ready; zero banks written"
                        .into(),
                );
            }
            Ok(Some((
                BANK_COUNT,
                "S3 dongle",
                status.selected_bank.unwrap_or(0),
            )))
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .flatten();
    if let Some(capacity) = dongle {
        return Ok(capacity);
    }

    with_pad(address, |pad| {
        Box::pin(async move {
            let info = pad.read_info().await.map_err(|e| e.to_string())?;
            if !cyberdeck_ble::info_protocol_compatible(&info) {
                return Err(format!("unsupported pad Info {info:?}; zero banks written"));
            }
            let banked = cyberdeck_ble::info_supports_banks(&info);
            let selected_bank = if banked {
                pad.read_selected_bank().await.map_err(|e| e.to_string())?
            } else {
                0
            };
            Ok((if banked { BANK_COUNT } else { 1 }, "BlueZ", selected_bank))
        })
    })
    .await
}

async fn restore_pad_bank(address: Option<String>, bank: u8) -> Result<(), String> {
    let _pad_io = PAD_IO.lock().await;
    restore_pad_bank_unlocked(address, bank).await
}

async fn restore_pad_bank_unlocked(address: Option<String>, bank: u8) -> Result<(), String> {
    validate_bank(bank)?;
    let restored_dongle = tokio::task::spawn_blocking(move || {
        with_dongle_session(|dongle| {
            let status = dongle.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok(false);
            }
            if status.protocol_compatible != Some(true) || status.slots_ready != Some(true) {
                return Err(
                    "dongle cannot restore BankSel because its v0.3 slots proxy is unavailable"
                        .into(),
                );
            }
            dongle.read_slots(bank).map_err(|e| e.to_string())?;
            Ok(true)
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .unwrap_or(false);
    if restored_dongle {
        return Ok(());
    }
    with_pad(address, |pad| {
        Box::pin(async move {
            if bank == 0 {
                pad.read_slots().await
            } else {
                pad.read_slots_for_bank(bank).await
            }
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
    })
    .await
}

#[tauri::command]
async fn export_profile(state: State<'_, Arc<AppState>>, name: String) -> Result<String, String> {
    let name = name.trim().replace(['/', '\\', '\0'], "_");
    if name.is_empty() {
        return Err("profile name required".into());
    }
    let store = state.store.lock().await;
    let dir = profiles_dir(&state.store_path);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    let profile = ProfileFile {
        schema_version: STORE_SCHEMA_VERSION,
        actions: store.actions.clone(),
        pad_bindings: store.pad_bindings.clone(),
        pad_preset_names: store.pad_preset_names.clone(),
        composers: store.composers.clone(),
        allowed_commands: store.allowed_commands.clone(),
        pad_slots: store.pad_slots.clone().map(PadSlotsWire::Banks),
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportProfileResult {
    store: Store,
    pad_write: Option<String>,
}

#[tauri::command]
async fn import_profile(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    path: String,
) -> Result<ImportProfileResult, String> {
    let state = state.inner().clone();
    with_store_pad_replacement(app, state.clone(), import_profile_inner(state, path)).await
}

async fn import_profile_inner(
    state: Arc<AppState>,
    path: String,
) -> Result<ImportProfileResult, String> {
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let profile: ProfileFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut store = state.store.lock().await;
    let (next, pad_write_scope) = apply_profile_file(&store, profile)?;
    next.save(&state.store_path)?;
    *store = next;
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
    let pad_write = match (pad_write_scope, out.pad_slots.as_ref()) {
        (ProfilePadWrite::None, _) | (_, None) => None,
        (scope, Some(banks)) if valid_pad_banks(&out.pad_slots) => {
            Some(match try_write_pad_banks_scope(None, banks, scope).await {
                Ok(message) => message,
                Err(e) => format!("profile imported locally; pad write failed: {e}"),
            })
        }
        _ => None,
    };
    let result = ImportProfileResult {
        store: out,
        pad_write,
    };
    Ok(result)
}

fn config_dir_from_store(store_path: &PathBuf) -> PathBuf {
    store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn apply_profile_file(
    store: &Store,
    profile: ProfileFile,
) -> Result<(Store, ProfilePadWrite), String> {
    if profile.schema_version > STORE_SCHEMA_VERSION {
        return Err(format!(
            "profile schema {} is newer than supported schema {STORE_SCHEMA_VERSION}",
            profile.schema_version
        ));
    }
    validate_action_ids(&profile.actions)?;
    validate_profile_pad_slots(&profile.pad_slots)?;

    let legacy_scope = profile.schema_version < STORE_SCHEMA_VERSION
        && !matches!(&profile.pad_slots, Some(PadSlotsWire::Banks(_)))
        || matches!(&profile.pad_slots, Some(PadSlotsWire::Flat(_)));
    let imported_bindings = migrate_pad_bindings(&profile.pad_bindings);
    let mut imported_actions = profile.actions;
    // An empty composer map is the legacy wire representation for defaults.
    // Seed those first, then reconcile retained bank dependencies exactly.
    // A retained custom `ai` therefore conflicts with the implicit default
    // instead of silently replacing it.
    let mut imported_composers = if profile.composers.is_empty() {
        default_composers()
    } else {
        profile.composers
    };
    if legacy_scope {
        let retained_action_ids: HashSet<_> = migrate_pad_bindings(&store.pad_bindings)
            .into_iter()
            .filter_map(|(key, action_id)| {
                matches!(parse_binding_key(&key), Some((bank, _, _, _)) if bank > 0)
                    .then_some(action_id)
            })
            .collect();
        let imported_by_id: HashMap<_, _> = imported_actions
            .iter()
            .map(|action| (action.id.as_str(), action))
            .collect();
        let mut retained_actions = Vec::new();
        let mut retained_composer_ids = HashSet::new();
        for retained in store
            .actions
            .iter()
            .filter(|action| retained_action_ids.contains(&action.id))
        {
            if let Some(composer_id) = action_composer_id(retained) {
                retained_composer_ids.insert(composer_id.to_string());
            }
            match imported_by_id.get(retained.id.as_str()) {
                Some(imported) if *imported != retained => {
                    return Err(format!(
                        "legacy profile action {:?} conflicts with the action retained by banks 1–{}",
                        retained.id,
                        BANK_COUNT - 1
                    ));
                }
                Some(_) => {}
                None => retained_actions.push(retained.clone()),
            }
        }
        imported_actions.extend(retained_actions);
        for composer_id in retained_composer_ids {
            let Some(retained) = store.composers.get(&composer_id) else {
                continue;
            };
            match imported_composers.get(&composer_id) {
                Some(imported) if imported != retained => {
                    return Err(format!(
                        "legacy profile composer {:?} conflicts with the config retained by banks 1–{}",
                        composer_id,
                        BANK_COUNT - 1
                    ));
                }
                Some(_) => {}
                None => {
                    imported_composers.insert(composer_id, retained.clone());
                }
            }
        }
    }
    let mut next = store.clone();
    next.actions = imported_actions;
    next.pad_bindings = if legacy_scope {
        let mut merged = migrate_pad_bindings(&next.pad_bindings);
        // A legacy profile only has authority over bank 0. Keep newer bank
        // bindings that the old format could not represent.
        merged
            .retain(|key, _| !matches!(parse_binding_key(key), Some((bank, _, _, _)) if bank == 0));
        for (key, value) in imported_bindings {
            if !matches!(parse_binding_key(&key), Some((bank, _, _, _)) if bank > 0) {
                merged.insert(key, value);
            }
        }
        merged
    } else {
        imported_bindings
    };
    next.pad_preset_names = profile.pad_preset_names;
    next.composers = imported_composers;
    // Profiles are portable data, not an authority grant. Imported commands
    // must be approved locally against their current text.
    let _ = profile.allowed_commands;
    next.allowed_commands.clear();
    next.approved_command_values.clear();
    // Older profiles can only replace bank 0. Banks 1–4 survive both locally
    // and on-device; a five-bank profile remains a full replacement.
    let pad_write = match profile.pad_slots {
        None => ProfilePadWrite::None,
        Some(PadSlotsWire::Flat(slots)) => {
            let mut banks = if valid_pad_banks(&next.pad_slots) {
                next.pad_slots
                    .clone()
                    .expect("valid_pad_banks established Some")
            } else {
                vec![empty_slot_bank(); BANK_COUNT]
            };
            banks[0] = slots;
            next.pad_slots = Some(banks);
            ProfilePadWrite::BankZero
        }
        Some(PadSlotsWire::Banks(banks)) => {
            next.pad_slots = Some(banks);
            ProfilePadWrite::AllBanks
        }
    };
    next.schema_version = STORE_SCHEMA_VERSION;
    next.normalize()?;
    Ok((next, pad_write))
}

#[tauri::command]
async fn git_sync_status(
    state: State<'_, Arc<AppState>>,
) -> Result<git_sync::GitSyncStatus, String> {
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
        schema_version: STORE_SCHEMA_VERSION,
        actions: store.actions.clone(),
        pad_bindings: store.pad_bindings.clone(),
        pad_preset_names: store.pad_preset_names.clone(),
        composers: store.composers.clone(),
        allowed_commands: store.allowed_commands.clone(),
        pad_slots: store.pad_slots.clone().map(PadSlotsWire::Banks),
    };
    drop(store);
    let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    let path = git_sync::write_profile_file(&dir, &name, &json)?;
    let push = git_sync::push_all(&dir)?;
    let mut settings = git_sync::load_settings(&dir);
    settings.active_profile = Some(git_sync::sanitize_profile_name(&name)?);
    git_sync::save_settings(&dir, &settings)?;
    let slot_note = match profile_has_all_banks(&profile.pad_slots) {
        true => format!(" · {} pad slots", BANK_COUNT * SLOT_COUNT),
        false => " · no pad slots in store (Refresh/Sync first)".into(),
    };
    Ok(format!("Wrote {}{slot_note} · {push}", path.display()))
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
    /// How many pad slots were in the applied profile (0 if none / older profile).
    pad_slot_count: usize,
    /// Outcome of writing slots to the pad, if attempted.
    pad_write: Option<String>,
}

#[tauri::command]
async fn git_sync_pull_apply(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<PullApplyResult, String> {
    let state = state.inner().clone();
    with_store_pad_replacement(app, state.clone(), git_sync_pull_apply_inner(state, name)).await
}

async fn git_sync_pull_apply_inner(
    state: Arc<AppState>,
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
    let (next, pad_write_scope) = apply_profile_file(&store, profile)?;
    next.save(&state.store_path)?;
    *store = next;
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

    let pad_slot_count = match pad_write_scope {
        ProfilePadWrite::None => 0,
        ProfilePadWrite::BankZero => SLOT_COUNT,
        ProfilePadWrite::AllBanks => BANK_COUNT * SLOT_COUNT,
    };
    let pad_write = match (pad_write_scope, out.pad_slots.as_ref()) {
        (ProfilePadWrite::None, _) | (_, None) => None,
        (scope, Some(banks)) if valid_pad_banks(&out.pad_slots) => {
            Some(match try_write_pad_banks_scope(None, banks, scope).await {
                Ok(msg) => msg,
                Err(e) => format!("pad write skipped: {e}"),
            })
        }
        _ => None,
    };

    let result = PullApplyResult {
        store: out,
        profile: clean_name,
        action_count,
        pull_message,
        unchanged,
        pad_slot_count,
        pad_write,
    };
    Ok(result)
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
async fn stop_macro_listen(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let _lifecycle = state.listen_lifecycle.lock().await;
    let _ = stop_listener_task(state.inner()).await;
    let _ = app.emit("pad-listening", false);
    Ok(())
}

fn listener_is_current(state: &AppState, generation: u64) -> bool {
    state.listen_generation.load(Ordering::SeqCst) == generation
}

async fn stop_listener_task(state: &Arc<AppState>) -> Option<ListenerRoute> {
    // Invalidate dispatch before signaling/awaiting the old task. An in-flight
    // CDC poll may finish, but its result can no longer execute an action.
    state.listen_generation.fetch_add(1, Ordering::SeqCst);
    let (stop, task, route) = {
        let mut control = state.listen_control.lock().await;
        (
            control.stop.take(),
            control.task.take(),
            control.route.take(),
        )
    };
    if let Some(tx) = stop {
        let _ = tx.send(());
    }
    if let Some(task) = task {
        let _ = task.await;
    }
    route
}

/// Profile replacement owns listener lifecycle as well as action dispatch.
/// The old generation is joined before mutation; a requested listener is
/// installed behind a barrier while dispatch remains closed. After reopen,
/// its startup path drains the S3 firmware queue before normal polling.
async fn with_store_pad_replacement<T, F>(
    app: AppHandle,
    state: Arc<AppState>,
    operation: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let replacement = state.action_dispatch.begin_replacement().await;
    let listener_lifecycle = state.listen_lifecycle.lock().await;
    let route = stop_listener_task(&state).await;
    let result = operation.await;
    let installed = if let Some(route) = route {
        install_macro_listener(app, state.clone(), route.address)
            .await
            .map(Some)
    } else {
        Ok(None)
    };
    // Installation creates a task blocked on a one-shot startup barrier. Open
    // dispatch first, then release that barrier synchronously: the fresh task
    // cannot issue a transport command while the replacement gate is closed.
    let restart = match installed {
        Ok(startup) => reopen_dispatch_and_release_listener(replacement, startup),
        Err(error) => {
            drop(replacement);
            Err(error)
        }
    };
    drop(listener_lifecycle);

    match (result, restart) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(format!(
            "profile applied but macro listener restart failed: {error}"
        )),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(restart_error)) => Err(format!(
            "{error}; macro listener restart also failed: {restart_error}"
        )),
    }
}

type ListenerStartup = tokio::sync::oneshot::Sender<()>;

fn release_listener_startup(startup: ListenerStartup) -> Result<(), String> {
    startup
        .send(())
        .map_err(|_| "listener task ended before transport startup was released".to_string())
}

fn reopen_dispatch_and_release_listener(
    replacement: ActionReplacementGuard,
    startup: Option<ListenerStartup>,
) -> Result<(), String> {
    // This ordering is the boundary invariant. No await/yield may be inserted
    // between reopening dispatch and releasing the installed listener.
    drop(replacement);
    match startup {
        Some(startup) => release_listener_startup(startup),
        None => Ok(()),
    }
}

#[tauri::command]
fn mode_constants() -> serde_json::Value {
    serde_json::json!({ "hid": MODE_HID, "macro": MODE_MACRO })
}

/// Fire a validated bank-scoped binding. Legacy "preset-action" input maps to bank 0.
async fn fire_binding_key(
    state: &Arc<AppState>,
    app: &AppHandle,
    key: &str,
    event_epoch: Option<u64>,
) -> String {
    let Some(_dispatch) = state.action_dispatch.try_enter_epoch(event_epoch) else {
        return "action dispatch dropped: profile replacement in progress".into();
    };
    let parsed = parse_binding_key(key);
    let (bank, preset, action, canonical_key) = match parsed {
        Some((bank, preset, action, _)) => (
            bank as u8,
            preset as u8,
            action as u8,
            PadSlots::bank_binding_key(bank, preset, action),
        ),
        None => {
            return format!("invalid binding key {key}");
        }
    };
    let mut store = state.store.lock().await;
    let action_id = store.pad_bindings.get(&canonical_key).cloned();
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
            format!("binding {canonical_key} points to missing action")
        }
    } else {
        format!("no binding for slot {canonical_key}")
    };
    let payload = MacroFiredPayload {
        bank,
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
    Ok(fire_binding_key(state.inner(), &app, &key, None).await)
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
        eprintln!("[mcc] localhost fire API on http://127.0.0.1:17321/fire/{{bank}}-{{p}}-{{a}} (legacy p-a = bank 0)");
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let mut buf = vec![0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let mut key = None;
            if let Some(rest) = req
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("POST /fire/"))
            {
                let path = rest.split_whitespace().next().unwrap_or("");
                key = Some(path.trim_matches('/').to_string());
            }
            let mut has_local_header = false;
            let mut has_browser_origin = false;
            for line in req.lines().skip(1) {
                let lower = line.to_ascii_lowercase();
                if lower.split_once(':').is_some_and(|(name, value)| {
                    name.trim() == "x-mcc-local" && value.trim() == "1"
                }) {
                    has_local_header = true;
                }
                if lower.starts_with("origin:") || lower.starts_with("referer:") {
                    has_browser_origin = true;
                }
            }
            let (status, body) = if has_browser_origin {
                (
                    "403 Forbidden",
                    "browser-origin requests are not accepted".into(),
                )
            } else if !has_local_header {
                ("403 Forbidden", "missing X-MCC-Local: 1".into())
            } else if let Some(k) = key {
                ("200 OK", fire_binding_key(&state, &app, &k, None).await)
            } else {
                (
                    "400 Bad Request",
                    "usage: POST /fire/0-2-0 with X-MCC-Local: 1 (legacy 2-0 maps to bank 0)"
                        .into(),
                )
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
Exec=curl -s -X POST -H X-MCC-Local:1 http://127.0.0.1:17321/fire/{slot}\n\
X-KDE-GlobalAccel-CommandShortcut=true\n\
X-KDE-Shortcuts={fkey}\n"
        );
        let _ = std::fs::write(&path, body);
    }
    // Reliable path: reuse Ctrl+Alt+1/2/3 (already bound in kglobalaccel).
    let openers = [
        (
            "net.local.open-task-app.desktop",
            "Ctrl+Alt+1",
            "2-0",
            "MCC fire 2-0",
        ),
        (
            "net.local.open-sysmon.desktop",
            "Ctrl+Alt+2",
            "2-1",
            "MCC fire 2-1",
        ),
        (
            "net.local.open-vscode.desktop",
            "Ctrl+Alt+3",
            "2-2",
            "MCC fire 2-2",
        ),
    ];
    for (file, chord, slot, name) in openers {
        let path = apps.join(file);
        let body = format!(
            "[Desktop Entry]\n\
Type=Application\n\
Name={name}\n\
NoDisplay=true\n\
StartupNotify=false\n\
Exec=curl -s -X POST -H X-MCC-Local:1 http://127.0.0.1:17321/fire/{slot}\n\
X-KDE-GlobalAccel-CommandShortcut=true\n\
X-KDE-Shortcuts={chord}\n"
        );
        let _ = std::fs::write(&path, body);
    }
    let _ = Command::new("kbuildsycoca6")
        .arg("--noincremental")
        .output();
    let _ = Command::new("kbuildsycoca5")
        .arg("--noincremental")
        .output();
}

fn main() {
    let store_path = config_path();
    let config_dir = store_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let store =
        Store::load(&store_path).unwrap_or_else(|e| panic!("{e}; store was left untouched"));
    let state = Arc::new(AppState {
        store_path,
        store: Mutex::new(store),
        listen_control: Mutex::new(ListenerControl::default()),
        listen_lifecycle: Mutex::new(()),
        listen_generation: AtomicU64::new(0),
        action_dispatch: Arc::new(ActionDispatchGate::default()),
        composer: Arc::new(Mutex::new(ComposerRuntime::default())),
        field_writer: Arc::new(FieldWriter::with_config_dir(Some(config_dir.clone()))),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            spawn_localhost_fire_api(handle.clone(), state.clone());
            install_kde_fire_shortcuts();
            let _ = ensure_ydotoold(Some(&config_dir));
            {
                let writer = state.field_writer.clone();
                let runtime = state.composer.clone();
                tauri::async_runtime::spawn(async move {
                    composer_write::writer_loop(writer, runtime).await;
                });
            }
            space_listen::spawn_space_listener(state.composer.clone(), state.field_writer.clone());
            // The frontend owns the single macro-listener startup after it has
            // refreshed the selected transport/address.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_store,
            save_store,
            pad_status,
            pad_set_bluez_enabled,
            pad_read_slots,
            pad_read_banks,
            pad_write_slots,
            pad_write_banks,
            pad_restore_bank,
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
    let _lifecycle = state.listen_lifecycle.lock().await;
    let _ = stop_listener_task(&state).await;
    let startup = install_macro_listener(app, state.clone(), address).await?;
    release_listener_startup(startup)
}

async fn install_macro_listener(
    app: AppHandle,
    state: Arc<AppState>,
    address: Option<String>,
) -> Result<ListenerStartup, String> {
    let generation = state.listen_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let event_epoch = state.action_dispatch.current_epoch();
    let (tx, rx_stop) = tokio::sync::oneshot::channel::<()>();
    let (startup_tx, startup_rx) = tokio::sync::oneshot::channel::<()>();
    let task_state = state.clone();
    let task_app = app.clone();
    let task_address = address.clone();
    let task = tauri::async_runtime::spawn(async move {
        let result = match startup_rx.await {
            Ok(()) if listener_is_current(&task_state, generation) => {
                run_macro_listener(
                    task_app.clone(),
                    task_state.clone(),
                    task_address,
                    generation,
                    event_epoch,
                    rx_stop,
                )
                .await
            }
            Ok(()) => Ok(()),
            Err(_) => Err("macro listener startup was canceled before transport access".into()),
        };
        if listener_is_current(&task_state, generation) {
            if let Err(error) = result {
                let _ = task_app.emit("pad-error", error);
            }
            let _ = task_app.emit("pad-listening", false);
        }
    });
    let mut control = state.listen_control.lock().await;
    control.stop = Some(tx);
    control.task = Some(task);
    control.route = Some(ListenerRoute { address });
    Ok(startup_tx)
}

async fn run_macro_listener(
    app: AppHandle,
    state: Arc<AppState>,
    address: Option<String>,
    generation: u64,
    event_epoch: u64,
    mut rx_stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let (dongle_route, drained_events) = tokio::task::spawn_blocking(|| {
        with_dongle_session(|dongle| {
            let status = dongle.status().map_err(|e| format!("dongle status: {e}"))?;
            if !status.connected {
                return Ok((false, 0));
            }
            if status.protocol_compatible != Some(true) {
                return Err(
                    "S3 dongle owns the pad but is not protocol v0.3; macro listener stopped"
                        .into(),
                );
            }
            if status.macro_ready != Some(true) {
                return Err("S3 dongle owns the pad but MacroEvent forwarding is not ready".into());
            }
            let drained = drain_dongle_macro_queue(dongle)?;
            Ok((true, drained))
        })
    })
    .await
    .map_err(|e| e.to_string())??
    .unwrap_or((false, 0));

    if !listener_is_current(&state, generation) {
        return Ok(());
    }

    if dongle_route {
        if drained_events > 0 {
            eprintln!(
                "[mcc] discarded {drained_events} buffered macro event(s) before listener epoch {event_epoch}"
            );
        }
        let _ = app.emit("pad-listening", true);
        let mut last_bank = None;
        loop {
            let polled = tokio::task::spawn_blocking(|| {
                with_dongle_session(|dongle| dongle.poll_events().map_err(|e| e.to_string()))?
                    .ok_or_else(|| "S3 dongle disappeared".to_string())
            })
            .await;

            // Cancellation invalidates this generation before awaiting the task,
            // so an in-flight poll can never dispatch after Stop/restart.
            if !listener_is_current(&state, generation) {
                break;
            }
            match polled {
                Ok(Ok(poll)) => {
                    if last_bank != Some(poll.selected_bank) {
                        last_bank = Some(poll.selected_bank);
                        let _ = app.emit("pad-bank-changed", poll.selected_bank);
                    }
                    if let Some(MacroEvent {
                        bank,
                        preset,
                        action,
                    }) = poll.macro_event
                    {
                        let key = PadSlots::bank_binding_key(
                            bank as usize,
                            preset as usize,
                            action as usize,
                        );
                        let _ = fire_binding_key(&state, &app, &key, Some(event_epoch)).await;
                    }
                }
                Ok(Err(e)) => {
                    let _ = app.emit("pad-error", format!("dongle macro poll: {e}"));
                }
                Err(e) => {
                    let _ = app.emit("pad-error", format!("dongle macro worker: {e}"));
                }
            }

            tokio::select! {
                biased;
                _ = &mut rx_stop => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
            }
        }
        return Ok(());
    }

    let pad = with_pad(address, |pad| Box::pin(async move { Ok(pad) })).await?;
    if !listener_is_current(&state, generation) {
        return Ok(());
    }
    let mut events = pad
        .subscribe_macro_events()
        .await
        .map_err(|e| e.to_string())?;
    if !listener_is_current(&state, generation) {
        return Ok(());
    }
    let mut bank_events = pad.subscribe_bank_events().await.ok();
    if let Ok(bank) = pad.read_selected_bank().await {
        if listener_is_current(&state, generation) {
            let _ = app.emit("pad-bank-changed", bank);
        }
    }

    if listener_is_current(&state, generation) {
        let _ = app.emit("pad-listening", true);
    }

    loop {
        tokio::select! {
            biased;
            _ = &mut rx_stop => break,
            ev = events.recv() => {
                let Some(MacroEvent { bank, preset, action }) = ev else { break };
                if !listener_is_current(&state, generation) {
                    break;
                }
                let key = PadSlots::bank_binding_key(
                    bank as usize,
                    preset as usize,
                    action as usize,
                );
                let _ = fire_binding_key(&state, &app, &key, Some(event_epoch)).await;
            }
            bank = async {
                if let Some(events) = bank_events.as_mut() {
                    events.recv().await
                } else {
                    std::future::pending::<Option<u8>>().await
                }
            } => {
                if !listener_is_current(&state, generation) {
                    break;
                }
                if let Some(bank) = bank {
                    let _ = app.emit("pad-bank-changed", bank);
                } else {
                    bank_events = None;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn action(id: &str, type_: &str, value: &str) -> Action {
        Action {
            id: id.into(),
            name: id.into(),
            category: "test".into(),
            description: String::new(),
            type_: type_.into(),
            value: value.into(),
            tags: Vec::new(),
            favorite: false,
            last_used: None,
            created_at: "test".into(),
        }
    }

    fn composer_config(commands: &[&str]) -> ComposerConfig {
        ComposerConfig {
            commands: commands.iter().map(|command| (*command).into()).collect(),
            ..ComposerConfig::default()
        }
    }

    #[test]
    fn flat_v02_store_migrates_slots_and_binding_to_bank_zero() {
        let flat = empty_slot_bank();
        let raw = serde_json::json!({
            "actions": [],
            "padBindings": {"2-0": "legacy-action"},
            "allowedCommands": [],
            "padSlots": flat,
        });
        let mut store: Store = serde_json::from_value(raw).unwrap();
        store.normalize().unwrap();
        assert_eq!(store.schema_version, STORE_SCHEMA_VERSION);
        let banks = store.pad_slots.unwrap();
        assert_eq!(banks.len(), BANK_COUNT);
        assert!(banks.iter().all(|bank| bank.len() == SLOT_COUNT));
        assert_eq!(
            store.pad_bindings.get("0-2-0"),
            Some(&"legacy-action".into())
        );
        assert!(!store.pad_bindings.contains_key("2-0"));
    }

    #[test]
    fn explicit_v03_binding_wins_over_legacy_collision() {
        let bindings = HashMap::from([
            ("2-0".into(), "legacy".into()),
            ("0-2-0".into(), "explicit".into()),
        ]);
        let migrated = migrate_pad_bindings(&bindings);
        assert_eq!(migrated.get("0-2-0"), Some(&"explicit".into()));
    }

    #[test]
    fn malformed_bank_shape_and_future_schema_are_rejected() {
        let raw = serde_json::json!({
            "actions": [],
            "padBindings": {},
            "allowedCommands": [],
            "padSlots": [[]],
        });
        assert!(serde_json::from_value::<Store>(raw).is_err());
        let mut future = Store::default();
        future.schema_version = STORE_SCHEMA_VERSION + 1;
        assert!(future.normalize().is_err());
    }

    #[test]
    fn empty_and_duplicate_action_ids_are_rejected() {
        let mut empty = Store::default();
        empty.actions.push(action("  ", "command", "true"));
        assert!(empty.normalize().unwrap_err().contains("empty id"));

        let mut duplicate = Store::default();
        duplicate.actions = vec![
            action("same", "command", "echo safe"),
            action("same", "command", "echo different"),
        ];
        assert!(duplicate.normalize().unwrap_err().contains("duplicate"));
    }

    #[test]
    fn command_approval_is_bound_to_exact_command_text() {
        let mut store = Store::default();
        store.actions = vec![action("cmd", "command", "echo reviewed")];
        store.allowed_commands.insert("cmd".into());
        store
            .approved_command_values
            .insert("cmd".into(), "echo reviewed".into());
        store.normalize().unwrap();
        assert!(store.command_is_approved(&store.actions[0]));

        store.actions[0].value = "echo changed".into();
        store.normalize().unwrap();
        assert!(!store.allowed_commands.contains("cmd"));
        assert!(!store.approved_command_values.contains_key("cmd"));
    }

    #[tokio::test]
    async fn replacement_gate_drains_in_flight_and_drops_new_dispatches() {
        let gate = Arc::new(ActionDispatchGate::default());
        let in_flight = gate.try_enter().expect("initial dispatch accepted");
        let replacement_gate = gate.clone();
        let replacement = tokio::spawn(async move { replacement_gate.begin_replacement().await });

        while gate.is_accepting() {
            tokio::task::yield_now().await;
        }
        assert!(
            gate.try_enter().is_none(),
            "closed gate must drop, not queue"
        );
        assert!(
            !replacement.is_finished(),
            "replacement must wait for the in-flight action"
        );

        drop(in_flight);
        let replacement = tokio::time::timeout(std::time::Duration::from_secs(1), replacement)
            .await
            .expect("replacement gate should drain")
            .expect("replacement task should not panic");
        assert!(
            gate.try_enter().is_none(),
            "gate stays closed for replacement"
        );

        drop(replacement);
        assert!(gate.try_enter().is_some(), "gate reopens after replacement");
    }

    #[tokio::test]
    async fn buffered_old_epoch_event_is_dropped_after_gate_reopens() {
        let gate = Arc::new(ActionDispatchGate::default());
        // Model an event already received by the old transport task but
        // delayed at the dispatch boundary until replacement has completed.
        let buffered_event_epoch = gate.current_epoch();
        let replacement = gate.begin_replacement().await;
        let replacement_epoch = gate.current_epoch();
        assert_ne!(buffered_event_epoch, replacement_epoch);

        drop(replacement);
        assert!(gate.is_accepting());
        assert!(
            gate.try_enter_epoch(Some(buffered_event_epoch)).is_none(),
            "an old buffered event must remain stale after reopen"
        );
        assert!(
            gate.try_enter_epoch(Some(replacement_epoch)).is_some(),
            "the restarted listener epoch may dispatch after reopen"
        );
    }

    #[tokio::test]
    async fn replacement_reopens_before_installed_listener_can_poll() {
        let gate = Arc::new(ActionDispatchGate::default());
        let replacement = gate.begin_replacement().await;
        let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();
        let listener_gate = gate.clone();
        let first_poll = tokio::spawn(async move {
            startup_rx.await.expect("startup released");
            listener_gate.is_accepting()
        });

        tokio::task::yield_now().await;
        assert!(!gate.is_accepting());
        assert!(
            !first_poll.is_finished(),
            "installed listener must remain blocked at the startup barrier"
        );

        reopen_dispatch_and_release_listener(replacement, Some(startup_tx)).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), first_poll)
                .await
                .expect("listener startup should complete")
                .expect("listener task should not panic"),
            "the first transport poll is released only after dispatch reopens"
        );
    }

    #[test]
    fn startup_drain_discards_events_through_the_empty_boundary() {
        // The second event models one arriving at the transport while startup
        // is already draining the pre-replacement buffer.
        let mut polls = std::collections::VecDeque::from([true, true, false]);
        let drained = drain_macro_queue_with(|| {
            polls
                .pop_front()
                .ok_or_else(|| "drain polled past empty boundary".to_string())
        })
        .unwrap();

        assert_eq!(drained, 2);
        assert!(polls.is_empty());
    }

    #[test]
    fn legacy_profile_updates_bank_zero_without_erasing_newer_banks() {
        let mut current = Store::default();
        let mut current_banks = vec![empty_slot_bank(); BANK_COUNT];
        current_banks[0][0].label = "old-zero".into();
        current_banks[1][0].label = "keep-one".into();
        current.pad_slots = Some(current_banks);
        current.pad_bindings = HashMap::from([
            ("0-2-0".into(), "old-zero-action".into()),
            ("1-2-0".into(), "keep-one-action".into()),
        ]);
        current.actions = vec![
            action("old-zero-action", "note", "old"),
            action("keep-one-action", "note", "keep"),
        ];

        let mut flat = empty_slot_bank();
        flat[0].label = "new-zero".into();
        let profile = ProfileFile {
            schema_version: 2,
            actions: vec![action("new-zero-action", "note", "new")],
            pad_bindings: HashMap::from([("2-0".into(), "new-zero-action".into())]),
            pad_preset_names: Vec::new(),
            composers: HashMap::new(),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(flat)),
        };

        let (applied, scope) = apply_profile_file(&current, profile).unwrap();
        assert_eq!(scope, ProfilePadWrite::BankZero);
        let banks = applied.pad_slots.unwrap();
        assert_eq!(banks[0][0].label, "new-zero");
        assert_eq!(banks[1][0].label, "keep-one");
        assert_eq!(
            applied.pad_bindings.get("0-2-0").map(String::as_str),
            Some("new-zero-action")
        );
        assert_eq!(
            applied.pad_bindings.get("1-2-0").map(String::as_str),
            Some("keep-one-action")
        );
        assert!(applied
            .actions
            .iter()
            .any(|action| action.id == "keep-one-action" && action.value == "keep"));
        assert!(!applied
            .actions
            .iter()
            .any(|action| action.id == "old-zero-action"));
    }

    #[test]
    fn legacy_profile_rejects_redefinition_used_by_retained_bank() {
        let mut current = Store {
            actions: vec![action("shared", "note", "retained definition")],
            ..Store::default()
        };
        current.pad_bindings.insert("4-2-0".into(), "shared".into());
        let profile = ProfileFile {
            schema_version: 2,
            actions: vec![action("shared", "note", "changed definition")],
            pad_bindings: HashMap::from([("2-0".into(), "shared".into())]),
            pad_preset_names: Vec::new(),
            composers: HashMap::new(),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(empty_slot_bank())),
        };

        let error = apply_profile_file(&current, profile).unwrap_err();
        assert!(error.contains("conflicts with the action retained by banks 1"));
    }

    #[test]
    fn legacy_profile_allows_identical_action_used_by_retained_bank() {
        let shared = action("shared", "note", "same definition");
        let mut current = Store {
            actions: vec![shared.clone()],
            ..Store::default()
        };
        current.pad_bindings.insert("2-2-0".into(), "shared".into());
        let profile = ProfileFile {
            schema_version: 2,
            actions: vec![shared],
            pad_bindings: HashMap::from([("2-0".into(), "shared".into())]),
            pad_preset_names: Vec::new(),
            composers: HashMap::new(),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(empty_slot_bank())),
        };

        let (applied, scope) = apply_profile_file(&current, profile).unwrap();
        assert_eq!(scope, ProfilePadWrite::BankZero);
        assert_eq!(
            applied
                .actions
                .iter()
                .filter(|action| action.id == "shared")
                .count(),
            1
        );
    }

    #[test]
    fn legacy_profile_preserves_composers_used_only_by_retained_banks() {
        let retained = composer_config(&["/retained", "/rotate"]);
        let mut current = Store {
            actions: vec![
                action("pick", "composer", "workflow"),
                action("commit", "composer-commit", "workflow"),
                action("reset", "composer-reset", "workflow"),
            ],
            composers: HashMap::from([("workflow".into(), retained.clone())]),
            ..Store::default()
        };
        current.pad_bindings = HashMap::from([
            ("1-2-0".into(), "pick".into()),
            ("2-2-0".into(), "commit".into()),
            ("4-2-0".into(), "reset".into()),
        ]);
        let profile = ProfileFile {
            schema_version: 2,
            actions: Vec::new(),
            pad_bindings: HashMap::new(),
            pad_preset_names: Vec::new(),
            composers: HashMap::new(),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(empty_slot_bank())),
        };

        let (applied, _) = apply_profile_file(&current, profile).unwrap();
        assert_eq!(applied.actions.len(), 3);
        assert_eq!(applied.composers.len(), 2);
        assert_eq!(applied.composers.get("ai"), default_composers().get("ai"));
        assert_eq!(applied.composers.get("workflow"), Some(&retained));
    }

    #[test]
    fn legacy_profile_rejects_conflicting_retained_composer_config() {
        let retained_action = action("pick", "composer", "workflow");
        let mut current = Store {
            actions: vec![retained_action.clone()],
            composers: HashMap::from([("workflow".into(), composer_config(&["/retained"]))]),
            ..Store::default()
        };
        current.pad_bindings.insert("3-2-0".into(), "pick".into());
        let profile = ProfileFile {
            schema_version: 2,
            actions: vec![retained_action],
            pad_bindings: HashMap::new(),
            pad_preset_names: Vec::new(),
            composers: HashMap::from([("workflow".into(), composer_config(&["/changed"]))]),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(empty_slot_bank())),
        };

        let error = apply_profile_file(&current, profile).unwrap_err();
        assert!(error.contains("composer \"workflow\" conflicts"));
    }

    #[test]
    fn legacy_empty_composers_reject_custom_retained_ai_conflict() {
        let mut current = Store {
            actions: vec![action("pick", "composer", "ai")],
            composers: HashMap::from([("ai".into(), composer_config(&["/custom-ai"]))]),
            ..Store::default()
        };
        current.pad_bindings.insert("1-2-0".into(), "pick".into());
        let profile = ProfileFile {
            schema_version: 2,
            actions: Vec::new(),
            pad_bindings: HashMap::new(),
            pad_preset_names: Vec::new(),
            composers: HashMap::new(),
            allowed_commands: HashSet::new(),
            pad_slots: Some(PadSlotsWire::Flat(empty_slot_bank())),
        };

        let error = apply_profile_file(&current, profile).unwrap_err();
        assert!(error.contains("composer \"ai\" conflicts"));
    }
}
