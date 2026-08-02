//! ydotool / ydotoold socket path and permission policy (P3-04).
//!
//! Never default to a world-shared `/tmp` socket. Prefer an owner-only
//! runtime path and spawn ydotoold with mode `0600`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Socket mode passed to `ydotoold -P` — owner-only (not world-writable).
pub const YDOTOOLD_SOCKET_MODE: &str = "0600";

/// Runtime subdirectory under `$XDG_RUNTIME_DIR` / config fallback.
const RUNTIME_APP_DIR: &str = "hk-operator";

/// Resolved socket location plus whether the operator owns the parent dir policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedYdotoolSocket {
    pub path: PathBuf,
    /// `true` when path came from `YDOTOOL_SOCKET` — do not chmod its parent.
    pub external_override: bool,
}

/// True when group/other have any access bits (world/group reachable).
pub fn socket_mode_allows_non_owner(mode: u32) -> bool {
    (mode & 0o077) != 0
}

pub fn metadata_allows_non_owner(meta: &fs::Metadata) -> bool {
    socket_mode_allows_non_owner(meta.permissions().mode())
}

/// Pure resolver: prefer override, then runtime dir, then config-dir fallback.
/// Never a world-shared `/tmp` default.
///
/// Production passes env via [`resolve_ydotool_socket`]; tests inject controlled
/// inputs without mutating process environment.
pub fn resolve_ydotool_socket_from(
    override_path: Option<OsString>,
    runtime_dir: Option<OsString>,
    config_dir: Option<&Path>,
) -> ResolvedYdotoolSocket {
    if let Some(p) = override_path {
        if !p.is_empty() {
            return ResolvedYdotoolSocket {
                path: PathBuf::from(p),
                external_override: true,
            };
        }
    }
    if let Some(runtime) = runtime_dir {
        if !runtime.is_empty() {
            return ResolvedYdotoolSocket {
                path: PathBuf::from(runtime)
                    .join(RUNTIME_APP_DIR)
                    .join("ydotool.sock"),
                external_override: false,
            };
        }
    }
    let path = if let Some(dir) = config_dir {
        dir.join("ydotool.sock")
    } else {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(RUNTIME_APP_DIR)
            .join("ydotool.sock")
    };
    ResolvedYdotoolSocket {
        path,
        external_override: false,
    }
}

/// Prefer `YDOTOOL_SOCKET`, then `$XDG_RUNTIME_DIR/<app>/ydotool.sock`,
/// then config-dir fallback.
pub fn resolve_ydotool_socket(config_dir: Option<&Path>) -> ResolvedYdotoolSocket {
    resolve_ydotool_socket_from(
        std::env::var_os("YDOTOOL_SOCKET"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        config_dir,
    )
}

/// Path-only helper (drops the override flag). Kept for callers that only need the path.
#[allow(dead_code)]
pub fn resolve_ydotool_socket_path(config_dir: Option<&Path>) -> PathBuf {
    resolve_ydotool_socket(config_dir).path
}

/// Args for `ydotoold` (`-p <path> -P 0600`).
pub fn ydotoold_spawn_args(sock: &Path) -> Vec<String> {
    vec![
        "-p".into(),
        sock.to_string_lossy().into_owned(),
        "-P".into(),
        YDOTOOLD_SOCKET_MODE.into(),
    ]
}

/// If an existing socket is group/world-accessible, remove it so we can recreate.
/// Returns true when a usable owner-only socket already exists.
///
/// `manage_parent_mode`: when true (internally generated paths), ensure the
/// parent directory exists and is `0700`. When false (`YDOTOOL_SOCKET`
/// override), only create the parent if missing — never chmod a shared path.
pub fn prepare_ydotool_socket(sock: &Path, manage_parent_mode: bool) -> Result<bool, String> {
    if let Some(parent) = sock.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        if manage_parent_mode {
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    if !sock.exists() {
        return Ok(false);
    }
    let meta = fs::metadata(sock).map_err(|e| e.to_string())?;
    if metadata_allows_non_owner(&meta) {
        eprintln!(
            "[mcc] removing non-owner-accessible ydotool socket {} (mode {:o})",
            sock.display(),
            meta.permissions().mode() & 0o777
        );
        fs::remove_file(sock).map_err(|e| e.to_string())?;
        return Ok(false);
    }
    Ok(true)
}

/// Ensure ydotoold is up with an owner-only socket (P3-04).
pub fn ensure_ydotoold(config_dir: Option<&Path>) -> PathBuf {
    let resolved = resolve_ydotool_socket(config_dir);
    match prepare_ydotool_socket(&resolved.path, !resolved.external_override) {
        Ok(true) => return resolved.path,
        Ok(false) => {}
        Err(e) => {
            eprintln!("[mcc] ydotool socket prepare failed: {e}");
            return resolved.path;
        }
    }
    let args = ydotoold_spawn_args(&resolved.path);
    let _ = Command::new("ydotoold")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    std::thread::sleep(std::time::Duration::from_millis(250));
    resolved.path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mode_0600_is_owner_only() {
        assert!(!socket_mode_allows_non_owner(0o600));
        assert!(!socket_mode_allows_non_owner(0o600 | 0o100000));
    }

    #[test]
    fn mode_0666_is_rejected() {
        assert!(socket_mode_allows_non_owner(0o666));
        assert!(socket_mode_allows_non_owner(0o606));
        assert!(socket_mode_allows_non_owner(0o660));
    }

    #[test]
    fn spawn_args_use_0600_not_0666() {
        let args = ydotoold_spawn_args(Path::new(
            "/run/user/1000/hk-operator/ydotool.sock",
        ));
        assert_eq!(args[0], "-p");
        assert_eq!(args[2], "-P");
        assert_eq!(args[3], "0600");
        assert!(!args.iter().any(|a| a == "0666"));
    }

    #[test]
    fn prepare_removes_world_accessible_socket() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hk-ydotool-test-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("ydotool.sock");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).mode(0o666);
            opts.open(&sock).unwrap();
        }
        assert!(prepare_ydotool_socket(&sock, true).unwrap() == false);
        assert!(!sock.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prepare_keeps_owner_only_socket() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hk-ydotool-test-ok-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("ydotool.sock");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).mode(0o600);
            opts.open(&sock).unwrap();
        }
        assert!(prepare_ydotool_socket(&sock, true).unwrap());
        assert!(sock.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prepare_skips_parent_chmod_for_external_override() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hk-ydotool-shared-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let sock = dir.join("ydotool.sock");
        assert!(!prepare_ydotool_socket(&sock, false).unwrap());
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "must not chmod shared parent for YDOTOOL_SOCKET");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prepare_chmods_parent_for_managed_paths() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hk-ydotool-managed-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let sock = dir.join("ydotool.sock");
        assert!(!prepare_ydotool_socket(&sock, true).unwrap());
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_dir_path_is_not_marked_external() {
        let r = resolve_ydotool_socket_from(
            None,
            None,
            Some(Path::new("/tmp/hk-cfg-test")),
        );
        assert!(!r.external_override);
        assert_eq!(r.path, PathBuf::from("/tmp/hk-cfg-test/ydotool.sock"));
    }

    #[test]
    fn runtime_dir_beats_config_and_is_managed() {
        let r = resolve_ydotool_socket_from(
            None,
            Some(OsString::from("/run/user/1000")),
            Some(Path::new("/tmp/hk-cfg-test")),
        );
        assert!(!r.external_override);
        assert_eq!(
            r.path,
            PathBuf::from("/run/user/1000")
                .join(RUNTIME_APP_DIR)
                .join("ydotool.sock")
        );
    }

    #[test]
    fn override_path_is_marked_external() {
        let r = resolve_ydotool_socket_from(
            Some(OsString::from("/some/shared/path/ydotool.sock")),
            Some(OsString::from("/run/user/1000")),
            Some(Path::new("/tmp/hk-cfg-test")),
        );
        assert!(r.external_override);
        assert_eq!(r.path, PathBuf::from("/some/shared/path/ydotool.sock"));
    }

    #[test]
    fn default_path_avoids_tmp_when_runtime_set() {
        let args = ydotoold_spawn_args(Path::new("/x/y.sock"));
        assert_eq!(args[3], YDOTOOLD_SOCKET_MODE);
        assert!(!args.iter().any(|a| a.contains("/tmp/.ydotool")));
    }
}
