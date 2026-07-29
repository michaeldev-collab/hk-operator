//! Localhost fire API request parsing and token auth (P3-01).
//! Pure helpers — no sockets here.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub const FIRE_TOKEN_FILE: &str = "fire_token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireRoute {
    /// Unauthenticated health probe (GET `/` or empty).
    Health,
    /// Authenticated binding fire (`POST /fire/{key}` only).
    Fire { key: String },
    /// `GET /fire/...` — rejected (use POST + token).
    FireGetRejected,
    Other,
}

/// Classify the first request line + whether a fire path was used.
pub fn classify_fire_route(req: &str) -> FireRoute {
    for line in req.lines() {
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("POST /fire/") {
            let path = rest.split_whitespace().next().unwrap_or("");
            let key = path.trim_matches('/').to_string();
            if key.is_empty() {
                return FireRoute::Other;
            }
            return FireRoute::Fire { key };
        }
        if line.starts_with("GET /fire/") {
            return FireRoute::FireGetRejected;
        }
        if line.starts_with("GET / ")
            || line.starts_with("GET /HTTP")
            || line == "GET /"
            || line.starts_with("GET /?")
        {
            return FireRoute::Health;
        }
        // First line only for method routing
        if line.starts_with("GET ") || line.starts_with("POST ") || line.starts_with("PUT ") {
            return FireRoute::Other;
        }
    }
    FireRoute::Other
}

/// Extract fire token from `Authorization: Bearer` or `X-HK-Fire-Token`.
pub fn extract_fire_token(req: &str) -> Option<String> {
    for line in req.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("x-hk-fire-token:") {
            let tok = trimmed.split_once(':')?.1.trim();
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
        if lower.starts_with("authorization:") {
            let value = trimmed.split_once(':')?.1.trim();
            let tok = value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))?;
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    None
}

pub fn fire_token_authorized(req: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    extract_fire_token(req)
        .map(|t| t == expected)
        .unwrap_or(false)
}

/// Load existing token or create a new hex token file (mode 0600).
pub fn load_or_create_fire_token(config_dir: &Path) -> Result<String, String> {
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let path = config_dir.join(FIRE_TOKEN_FILE);
    if path.is_file() {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let token = raw.trim().to_string();
        if !token.is_empty() && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(token);
        }
    }
    let token = generate_fire_token()?;
    write_fire_token_file(&path, &token)?;
    Ok(token)
}

pub fn generate_fire_token() -> Result<String, String> {
    let mut buf = [0u8; 16];
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn write_fire_token_file(path: &Path, token: &str) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    {
        let mut f = opts.open(path).map_err(|e| e.to_string())?;
        use std::io::Write;
        f.write_all(token.as_bytes()).map_err(|e| e.to_string())?;
        f.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    let mut perms = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
    Ok(())
}

/// curl Exec line for KDE shortcuts (hex token is shell-safe).
pub fn fire_curl_exec(slot: &str, token: &str) -> String {
    format!(
        "curl -s -X POST -H \"X-HK-Fire-Token: {token}\" http://127.0.0.1:17321/fire/{slot}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn classifies_health_and_fire_post() {
        assert_eq!(
            classify_fire_route("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            FireRoute::Health
        );
        assert_eq!(
            classify_fire_route("POST /fire/2-0 HTTP/1.1\r\n\r\n"),
            FireRoute::Fire {
                key: "2-0".into()
            }
        );
        assert_eq!(
            classify_fire_route("GET /fire/2-0 HTTP/1.1\r\n\r\n"),
            FireRoute::FireGetRejected
        );
    }

    #[test]
    fn token_from_header_and_bearer() {
        let req = "POST /fire/2-0 HTTP/1.1\r\nX-HK-Fire-Token: abcdef0123456789\r\n\r\n";
        assert_eq!(
            extract_fire_token(req).as_deref(),
            Some("abcdef0123456789")
        );
        let req2 = "POST /fire/2-0 HTTP/1.1\r\nAuthorization: Bearer deadbeef\r\n\r\n";
        assert_eq!(extract_fire_token(req2).as_deref(), Some("deadbeef"));
        assert!(!fire_token_authorized(req2, "other"));
        assert!(fire_token_authorized(req2, "deadbeef"));
    }

    #[test]
    fn curl_exec_includes_header() {
        let e = fire_curl_exec("2-1", "aa11");
        assert!(e.contains("X-HK-Fire-Token: aa11"));
        assert!(e.contains("/fire/2-1"));
        assert!(e.contains("-X POST"));
    }

    #[test]
    fn empty_expected_token_never_authorizes() {
        assert!(!fire_token_authorized(
            "POST /fire/0-0 HTTP/1.1\r\nX-HK-Fire-Token: x\r\n\r\n",
            ""
        ));
    }

    /// P3-02 helper coverage lives with profile apply — keep a smoke import here.
    #[test]
    fn retain_allowlist_intersection_semantics() {
        let mut allowed: HashSet<String> = ["a1", "a2", "gone"].iter().map(|s| (*s).into()).collect();
        let actions: HashSet<String> = ["a1", "a2", "a3"].iter().map(|s| (*s).into()).collect();
        allowed.retain(|id| actions.contains(id));
        assert!(allowed.contains("a1"));
        assert!(!allowed.contains("gone"));
        assert!(!allowed.contains("a3")); // not previously allowed
    }
}
