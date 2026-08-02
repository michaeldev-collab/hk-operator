//! Host keys while composer is active:
//! - Space → commit current pick (stack separator)
//!
//! New-loop reset is pad-only (Preset 3 · B4 / `composer-reset`).
//! Enter and Escape are left alone for Cursor slash UI.

use crate::composer::{any_picking, commit_all_picking, ComposerRuntime};
use crate::composer_write::FieldWriter;
use evdev::{Device, InputEventKind, Key};
use std::fs;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

fn set_nonblocking(dev: &Device) {
    let fd = dev.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

fn keyboard_paths() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in entries.flatten() {
        let path = ent.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("event") {
            continue;
        }
        let Ok(dev) = Device::open(&path) else {
            continue;
        };
        let has_space = dev
            .supported_keys()
            .is_some_and(|keys| keys.contains(Key::KEY_SPACE));
        if !has_space {
            continue;
        }
        let dname = dev.name().unwrap_or("").to_ascii_lowercase();
        if dname.contains("mouse") && !dname.contains("keyboard") {
            continue;
        }
        if dname.contains("ydotool") {
            continue;
        }
        out.push(path);
    }
    out
}

fn open_keyboards() -> Vec<Device> {
    let mut devices = Vec::new();
    for path in keyboard_paths() {
        match Device::open(&path) {
            Ok(d) => {
                set_nonblocking(&d);
                eprintln!(
                    "[composer-space] listening on {} ({})",
                    path.display(),
                    d.name().unwrap_or("?")
                );
                devices.push(d);
            }
            Err(e) => eprintln!("[composer-space] skip {}: {e}", path.display()),
        }
    }
    if devices.is_empty() {
        eprintln!("[composer-space] no readable keyboard — Space commit disabled");
    }
    devices
}

fn space_poll_loop(runtime: Arc<Mutex<ComposerRuntime>>, writer: Arc<FieldWriter>) {
    let mut devices = open_keyboards();
    let mut refresh_at = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if devices.is_empty() || std::time::Instant::now() >= refresh_at {
            devices = open_keyboards();
            refresh_at = std::time::Instant::now() + Duration::from_secs(10);
            if devices.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }

        let picking = any_picking(&runtime.blocking_lock());
        let mut saw_space = false;
        for dev in devices.iter_mut() {
            match dev.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        // Always drain; only act on Space while picking.
                        if picking
                            && matches!(ev.kind(), InputEventKind::Key(Key::KEY_SPACE))
                            && ev.value() == 1
                        {
                            saw_space = true;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
        }

        if saw_space {
            let mut rt = runtime.blocking_lock();
            let ids = commit_all_picking(&mut rt);
            drop(rt);
            if !ids.is_empty() {
                let fw = writer.clone();
                tauri::async_runtime::spawn(async move {
                    fw.clear_preview().await;
                });
                eprintln!("[composer-space] committed via Space: {ids:?}");
                let _ = Command::new("notify-send")
                    .args([
                        "-a",
                        "MCC Pad",
                        "composer committed",
                        "Space — pick locked. Double-tap to stack · P3 B4 for new loop.",
                    ])
                    .status();
            }
        }
        std::thread::sleep(Duration::from_millis(if picking { 8 } else { 40 }));
    }
}

pub fn spawn_space_listener(runtime: Arc<Mutex<ComposerRuntime>>, writer: Arc<FieldWriter>) {
    std::thread::Builder::new()
        .name("composer-space".into())
        .spawn(move || space_poll_loop(runtime, writer))
        .expect("spawn composer-space thread");
}
