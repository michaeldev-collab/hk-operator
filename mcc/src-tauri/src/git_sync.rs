//! Git-backed profile sync (`~/.config/<app>/hk-config`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitSyncSettings {
    pub remote: Option<String>,
    pub last_pull_at: Option<u64>,
    pub last_push_at: Option<u64>,
    pub active_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhAuthStatus {
    pub logged_in: bool,
    pub user: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSyncStatus {
    pub config_dir: String,
    pub repo_path: String,
    pub repo_exists: bool,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
    pub profiles: Vec<String>,
    pub auth: GhAuthStatus,
    pub settings: GitSyncSettings,
}

fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    // Prefer GIT_SSH_COMMAND from the environment. Otherwise optional
    // HK_GIT_SSH_IDENTITY may point at an identity file (operator-local).
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        if let Ok(identity) = std::env::var("HK_GIT_SSH_IDENTITY") {
            let key = PathBuf::from(identity.trim());
            if key.is_file() {
                let known = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".ssh/known_hosts");
                let ssh = format!(
                    "ssh -F /dev/null -o IdentitiesOnly=yes -i {} -o UserKnownHostsFile={}",
                    key.display(),
                    known.display()
                );
                cmd.env("GIT_SSH_COMMAND", ssh);
            }
        }
    }
    let out = cmd.output().map_err(|e| format!("git failed to start: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(if stderr.is_empty() {
            format!("git {:?} failed", args)
        } else {
            stderr
        });
    }
    Ok(stdout)
}

fn run_capture(bin: &str, args: &[&str]) -> Result<(i32, String, String), String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin} failed to start: {e}"))?;
    Ok((
        out.status.code().unwrap_or(1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub fn hk_config_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("hk-config")
}

pub fn profiles_repo_dir(config_dir: &Path) -> PathBuf {
    hk_config_dir(config_dir).join("profiles")
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load_settings(config_dir: &Path) -> GitSyncSettings {
    let path = settings_path(config_dir);
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => GitSyncSettings::default(),
    }
}

pub fn save_settings(config_dir: &Path, settings: &GitSyncSettings) -> Result<(), String> {
    let path = settings_path(config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Merge with any unknown keys already present
    let mut map: serde_json::Map<String, serde_json::Value> = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => serde_json::Map::new(),
    };
    if let Some(r) = &settings.remote {
        map.insert("remote".into(), serde_json::Value::String(r.clone()));
        map.insert("gitRemote".into(), serde_json::Value::String(r.clone()));
    }
    if let Some(t) = settings.last_pull_at {
        map.insert("lastPullAt".into(), serde_json::json!(t));
    }
    if let Some(t) = settings.last_push_at {
        map.insert("lastPushAt".into(), serde_json::json!(t));
    }
    if let Some(p) = &settings.active_profile {
        map.insert("activeProfile".into(), serde_json::Value::String(p.clone()));
    }
    let json = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn list_repo_profiles(config_dir: &Path) -> Vec<String> {
    let dir = profiles_repo_dir(config_dir);
    let mut names = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn gh_auth_status() -> GhAuthStatus {
    match run_capture("gh", &["auth", "status", "--hostname", "github.com"]) {
        Ok((0, stdout, stderr)) => {
            let detail = if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            };
            let user = detail
                .lines()
                .find_map(|l| {
                    l.split_once("account ")
                        .map(|(_, rest)| rest.split_whitespace().next().unwrap_or("").to_string())
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| {
                    // fallback: gh api user
                    run_capture("gh", &["api", "user", "-q", ".login"])
                        .ok()
                        .filter(|(c, _, _)| *c == 0)
                        .map(|(_, o, _)| o.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
            GhAuthStatus {
                logged_in: true,
                user,
                detail: detail.trim().to_string(),
            }
        }
        Ok((_, stdout, stderr)) => GhAuthStatus {
            logged_in: false,
            user: None,
            detail: format!("{}{}", stdout, stderr).trim().to_string(),
        },
        Err(e) => GhAuthStatus {
            logged_in: false,
            user: None,
            detail: e,
        },
    }
}

pub fn open_gh_login() -> Result<String, String> {
    // Launch an interactive login in a terminal when possible.
    let script = "gh auth login -h github.com -p ssh -w; echo; read -n1 -p 'Press any key to close…'";
    let terminals: &[(&str, &[&str])] = &[
        ("konsole", &["-e", "bash", "-lc", script]),
        ("gnome-terminal", &["--", "bash", "-lc", script]),
        ("xfce4-terminal", &["-e", "bash -lc"]),
        ("xterm", &["-e", "bash", "-lc", script]),
        ("kitty", &["bash", "-lc", script]),
    ];
    for (bin, _args) in terminals {
        if Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let spawn = match *bin {
                "konsole" => Command::new("konsole")
                    .args(["-e", "bash", "-lc", script])
                    .spawn(),
                "gnome-terminal" => Command::new("gnome-terminal")
                    .args(["--", "bash", "-lc", script])
                    .spawn(),
                "kitty" => Command::new("kitty").args(["bash", "-lc", script]).spawn(),
                "xterm" => Command::new("xterm")
                    .args(["-e", "bash", "-lc", script])
                    .spawn(),
                "xfce4-terminal" => Command::new("xfce4-terminal")
                    .args(["-e", &format!("bash -lc '{script}'")])
                    .spawn(),
                _ => continue,
            };
            spawn.map_err(|e| e.to_string())?;
            return Ok(format!("Opened {bin} for gh auth login — finish in that window"));
        }
    }
    Err(
        "No terminal found. Run in a shell:\n  gh auth login -h github.com -p ssh -w"
            .into(),
    )
}

pub fn ensure_local_repo(config_dir: &Path) -> Result<PathBuf, String> {
    let repo = hk_config_dir(config_dir);
    let profiles = repo.join("profiles");
    std::fs::create_dir_all(&profiles).map_err(|e| e.to_string())?;
    if !repo.join(".git").exists() {
        run_git(&repo, &["init", "-b", "main"])?;
        let readme = "# hk-config\n\nPortable MCC / HK Operator profiles.\n\nPut profile JSON files in `profiles/`.\n";
        std::fs::write(repo.join("README.md"), readme).map_err(|e| e.to_string())?;
        let gitignore = "settings.local.json\n*.bak\n";
        std::fs::write(repo.join(".gitignore"), gitignore).map_err(|e| e.to_string())?;
        run_git(&repo, &["add", "README.md", ".gitignore", "profiles"])?;
        let _ = run_git(
            &repo,
            &[
                "commit",
                "-m",
                "chore: initialize hk-config profiles repo",
            ],
        );
    }
    Ok(repo)
}

pub fn set_remote(config_dir: &Path, remote: &str) -> Result<(), String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return Err("remote URL required".into());
    }
    let repo = ensure_local_repo(config_dir)?;
    // Add or update origin
    let has_origin = run_git(&repo, &["remote"]).map(|s| s.lines().any(|l| l.trim() == "origin"));
    match has_origin {
        Ok(true) => {
            run_git(&repo, &["remote", "set-url", "origin", remote])?;
        }
        Ok(false) => {
            run_git(&repo, &["remote", "add", "origin", remote])?;
        }
        Err(_) => {
            let _ = run_git(&repo, &["remote", "remove", "origin"]);
            run_git(&repo, &["remote", "add", "origin", remote])?;
        }
    }
    let mut settings = load_settings(config_dir);
    settings.remote = Some(remote.to_string());
    save_settings(config_dir, &settings)?;
    Ok(())
}

pub fn pull(config_dir: &Path) -> Result<String, String> {
    let repo = ensure_local_repo(config_dir)?;
    let settings = load_settings(config_dir);
    if settings.remote.is_none() {
        // try reading origin
        let origin = run_git(&repo, &["remote", "get-url", "origin"]).ok();
        if origin.is_none() {
            return Err("No remote set — add a GitHub URL first".into());
        }
    }
    // fetch + ff-only pull
    let msg = match run_git(&repo, &["pull", "--ff-only", "origin", "main"]) {
        Ok(s) => {
            if s.is_empty() {
                "Already up to date".into()
            } else {
                s
            }
        }
        Err(e) => {
            // try master
            run_git(&repo, &["pull", "--ff-only", "origin", "master"]).map_err(|_| e)?
        }
    };
    let mut settings = load_settings(config_dir);
    settings.last_pull_at = Some(now_secs());
    save_settings(config_dir, &settings)?;
    Ok(msg)
}

pub fn push_all(config_dir: &Path) -> Result<String, String> {
    let repo = ensure_local_repo(config_dir)?;
    run_git(&repo, &["add", "profiles", "README.md", ".gitignore"])?;
    let status = run_git(&repo, &["status", "--porcelain"])?;
    if !status.trim().is_empty() {
        run_git(
            &repo,
            &[
                "commit",
                "-m",
                "chore: sync MCC profiles",
            ],
        )?;
    }
    let msg = run_git(&repo, &["push", "-u", "origin", "HEAD"]).unwrap_or_else(|e| {
        // first push might need main
        run_git(&repo, &["push", "-u", "origin", "main"]).unwrap_or(e)
    });
    let mut settings = load_settings(config_dir);
    settings.last_push_at = Some(now_secs());
    save_settings(config_dir, &settings)?;
    Ok(if msg.is_empty() {
        "Pushed".into()
    } else {
        msg
    })
}

pub fn write_profile_file(config_dir: &Path, name: &str, json: &str) -> Result<PathBuf, String> {
    let name = sanitize_profile_name(name)?;
    let dir = profiles_repo_dir(config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn read_profile_file(config_dir: &Path, name: &str) -> Result<String, String> {
    let name = sanitize_profile_name(name)?;
    let path = profiles_repo_dir(config_dir).join(format!("{name}.json"));
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

pub fn sanitize_profile_name(name: &str) -> Result<String, String> {
    let name = name.trim().replace(['/', '\\', '\0'], "_");
    if name.is_empty() {
        return Err("profile name required".into());
    }
    Ok(name)
}

pub fn create_github_repo(config_dir: &Path, repo_name: &str, private: bool) -> Result<String, String> {
    let auth = gh_auth_status();
    if !auth.logged_in {
        return Err("GitHub auth required — click Login first".into());
    }
    let repo_name = repo_name.trim();
    if repo_name.is_empty() {
        return Err("repo name required".into());
    }
    let repo = ensure_local_repo(config_dir)?;
    let visibility = if private { "--private" } else { "--public" };
    // gh repo create NAME --private --source=. --remote=origin --push
    let (code, stdout, stderr) = run_capture(
        "gh",
        &[
            "repo",
            "create",
            repo_name,
            visibility,
            "--source",
            repo.to_str().unwrap_or("."),
            "--remote",
            "origin",
            "--push",
        ],
    )?;
    if code != 0 {
        // maybe repo exists — try set remote from user/name
        let user = auth.user.clone().unwrap_or_default();
        if !user.is_empty() {
            let url = format!("git@github.com:{user}/{repo_name}.git");
            set_remote(config_dir, &url)?;
            let push = push_all(config_dir)?;
            return Ok(format!(
                "Repo may already exist; set origin to {url} and pushed.\n{push}\n{stderr}"
            ));
        }
        return Err(format!("gh repo create failed: {stderr} {stdout}"));
    }
    let remote = run_git(&repo, &["remote", "get-url", "origin"]).unwrap_or_else(|_| {
        auth.user
            .as_ref()
            .map(|u| format!("git@github.com:{u}/{repo_name}.git"))
            .unwrap_or_default()
    });
    let mut settings = load_settings(config_dir);
    settings.remote = Some(remote.clone());
    settings.last_push_at = Some(now_secs());
    save_settings(config_dir, &settings)?;
    Ok(format!("Created and pushed: {remote}\n{stdout}"))
}

pub fn status(config_dir: &Path) -> GitSyncStatus {
    let repo = hk_config_dir(config_dir);
    let repo_exists = repo.join(".git").exists();
    let mut settings = load_settings(config_dir);
    // Prefer serde fields; also accept gitRemote alias from older settings
    if settings.remote.is_none() {
        if let Ok(raw) = std::fs::read_to_string(settings_path(config_dir)) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(r) = v.get("gitRemote").and_then(|x| x.as_str()) {
                    settings.remote = Some(r.to_string());
                }
            }
        }
    }
    let remote = if repo_exists {
        run_git(&repo, &["remote", "get-url", "origin"])
            .ok()
            .or(settings.remote.clone())
    } else {
        settings.remote.clone()
    };
    let branch = if repo_exists {
        run_git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()
    } else {
        None
    };
    let dirty = if repo_exists {
        run_git(&repo, &["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    } else {
        false
    };
    GitSyncStatus {
        config_dir: config_dir.display().to_string(),
        repo_path: repo.display().to_string(),
        repo_exists,
        remote,
        branch,
        dirty,
        profiles: list_repo_profiles(config_dir),
        auth: gh_auth_status(),
        settings,
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
