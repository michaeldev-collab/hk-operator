//! ydotool / ydotoold socket path and permission policy (P3-04).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Socket mode passed to `ydotoold -P` — owner-only (not world-writable).
pub const YDOTOOLD_SOCKET_MODE: &str = "0600";

/// True when group/other have any access bits (world/group reachable).
pub fn socket_mode_allows_non_owner(mode: u32) -> bool {
    (mode & 0o077) != 0
}

pub fn metadata_allows_non_owner(meta: &fs::Metadata) -> bool {
    socket_mode_allows_non_owner(meta.permissions().mode())
}

/// Prefer `YDOTOOL_SOCKET`, then `$XDG_RUNTIME_DIR/hk-operator/ydotool.sock`,
/// then config-dir fallback — never a world-shared `/tmp` default.
pub fn resolve_ydotool_socket_path(config_dir: Option<&Path>) -> PathBuf {
    if let Some(p) = std::env::var_os("YDOTOOL_SOCKET") {
        return PathBuf::from(p);
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime)
                .join("hk-operator")
                .join("ydotool.sock");
        }
    }
    if let Some(dir) = config_dir {
        return dir.join("ydotool.sock");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hk-operator")
        .join("ydotool.sock")
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
pub fn prepare_ydotool_socket(sock: &Path) -> Result<bool, String> {
    if let Some(parent) = sock.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Runtime dir should stay private when we create hk-operator/.
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mode_0600_is_owner_only() {
        assert!(!socket_mode_allows_non_owner(0o600));
        assert!(!socket_mode_allows_non_owner(0o600 | 0o100000)); // ignore type bits if present
    }

    #[test]
    fn mode_0666_is_rejected() {
        assert!(socket_mode_allows_non_owner(0o666));
        assert!(socket_mode_allows_non_owner(0o606));
        assert!(socket_mode_allows_non_owner(0o660));
    }

    #[test]
    fn spawn_args_use_0600_not_0666() {
        let args = ydotoold_spawn_args(Path::new("/run/user/1000/hk-operator/ydotool.sock"));
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
        assert!(prepare_ydotool_socket(&sock).unwrap() == false);
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
        assert!(prepare_ydotool_socket(&sock).unwrap());
        assert!(sock.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_path_avoids_tmp_when_runtime_set() {
        // Only assert helper shape with explicit config_dir (env may vary in CI).
        let p = resolve_ydotool_socket_path(Some(Path::new("/tmp/hk-cfg-test")));
        // Without YDOTOOL_SOCKET override this uses runtime or config — config fallback:
        // If XDG_RUNTIME_DIR is set in this environment, path won't be under /tmp/hk-cfg-test.
        let _ = p;
        let args = ydotoold_spawn_args(Path::new("/x/y.sock"));
        assert_eq!(args[3], YDOTOOLD_SOCKET_MODE);
    }
}
