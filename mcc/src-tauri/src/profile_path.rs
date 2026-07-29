//! Confine profile import paths (P3-06).

use std::fs;
use std::path::{Path, PathBuf};

/// Expand a leading `~/` using the process home dir.
pub fn expand_user_path(path: &str) -> PathBuf {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

fn path_has_unsafe_chars(s: &str) -> bool {
    s.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
}

/// True when `child` is equal to `root` or a descendant (component-wise).
pub fn path_is_within(child: &Path, root: &Path) -> bool {
    let child_comp: Vec<_> = child.components().collect();
    let root_comp: Vec<_> = root.components().collect();
    if root_comp.len() > child_comp.len() {
        return false;
    }
    child_comp
        .iter()
        .zip(root_comp.iter())
        .all(|(c, r)| c == r)
        && child_comp.len() >= root_comp.len()
}

fn ensure_root_canonical(root: &Path) -> Result<PathBuf, String> {
    if !root.exists() {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
    }
    fs::canonicalize(root).map_err(|e| format!("cannot resolve allowed profile dir: {e}"))
}

/// Resolve an import path and require it to live under one of `allowed_roots`.
///
/// Bare names like `dev.json` or `dev` resolve under the first allowed root.
pub fn resolve_confined_profile_path(
    requested: &str,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    if allowed_roots.is_empty() {
        return Err("no allowed profile directories configured".into());
    }
    let requested = requested.trim();
    if requested.is_empty() {
        return Err("profile path required".into());
    }
    if path_has_unsafe_chars(requested) {
        return Err("profile path contains invalid characters".into());
    }

    let expanded = if !requested.contains('/') && !requested.contains('\\') {
        // Basename / stem only → first profiles dir
        let name = if requested.ends_with(".json") {
            requested.to_string()
        } else {
            format!("{requested}.json")
        };
        allowed_roots[0].join(name)
    } else {
        expand_user_path(requested)
    };

    if expanded
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        != Some(true)
    {
        return Err("profile import must be a .json file".into());
    }

    let canon = if expanded.exists() {
        fs::canonicalize(&expanded).map_err(|e| e.to_string())?
    } else {
        return Err(format!("profile file not found: {}", expanded.display()));
    };

    for root in allowed_roots {
        let root_canon = ensure_root_canonical(root)?;
        if path_is_within(&canon, &root_canon) {
            return Ok(canon);
        }
    }

    Err(
        "profile import path not allowed — use ~/.config/hk-operator/profiles/ or hk-config/profiles/"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_roots() -> (PathBuf, PathBuf, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("hk-profile-path-{stamp}"));
        let profiles = base.join("profiles");
        let git_profiles = base.join("hk-config").join("profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::create_dir_all(&git_profiles).unwrap();
        (base, profiles, git_profiles)
    }

    #[test]
    fn accepts_file_inside_profiles() {
        let (base, profiles, git_profiles) = tmp_roots();
        let file = profiles.join("dev.json");
        fs::write(&file, "{}").unwrap();
        let got =
            resolve_confined_profile_path(file.to_str().unwrap(), &[profiles.clone(), git_profiles])
                .unwrap();
        assert_eq!(got, fs::canonicalize(&file).unwrap());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn accepts_basename_under_first_root() {
        let (base, profiles, git_profiles) = tmp_roots();
        fs::write(profiles.join("dev.json"), "{}").unwrap();
        let got =
            resolve_confined_profile_path("dev", &[profiles.clone(), git_profiles]).unwrap();
        assert!(got.ends_with("dev.json"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn rejects_outside_and_traversal() {
        let (base, profiles, git_profiles) = tmp_roots();
        let outside = base.join("secret.json");
        fs::write(&outside, "x").unwrap();
        assert!(resolve_confined_profile_path(
            outside.to_str().unwrap(),
            &[profiles.clone(), git_profiles.clone()]
        )
        .is_err());

        // Escape via .. should fail once canonicalized outside root
        let escape = profiles.join("..").join("secret.json");
        assert!(resolve_confined_profile_path(
            escape.to_str().unwrap(),
            &[profiles, git_profiles]
        )
        .is_err());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn rejects_non_json() {
        let (base, profiles, git_profiles) = tmp_roots();
        let file = profiles.join("notes.txt");
        fs::write(&file, "x").unwrap();
        assert!(resolve_confined_profile_path(
            file.to_str().unwrap(),
            &[profiles, git_profiles]
        )
        .is_err());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn path_is_within_checks_prefix() {
        assert!(path_is_within(
            Path::new("/home/u/.config/hk-operator/profiles/a.json"),
            Path::new("/home/u/.config/hk-operator/profiles")
        ));
        assert!(!path_is_within(
            Path::new("/home/u/.config/hk-operator/store.json"),
            Path::new("/home/u/.config/hk-operator/profiles")
        ));
    }
}
