//! Update check: ask GitHub for the latest release tag and compare it with the
//! running version.
//!
//! Zero dependencies — on Windows a hand-written WinHTTP call; on Linux the
//! system `curl` binary. Either way the "nothing from crates.io" promise
//! holds. This is the *only* network code in PeekGuard: a single
//! unauthenticated GET to the public GitHub Releases API, carrying nothing
//! about the machine or the user. It is opt-out via `check_update = false` in
//! the config; see the README's privacy section.
//!
//! The call blocks on the network, so it must run on a background thread. The
//! result is posted back to the UI thread by the caller (`main.rs`).

#[cfg(windows)]
use crate::win::*;
#[cfg(windows)]
use std::ffi::c_void;

/// The running version, read straight from `Cargo.toml` so there is exactly one
/// source of truth — bump the manifest and the About window, the tray menu, and
/// the update comparison all follow.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const REPO_HOST: &str = "api.github.com";
const REPO_LATEST_PATH: &str = "/repos/Reance195/peekguard/releases/latest";
const FALLBACK_URL: &str = "https://github.com/Reance195/peekguard/releases/latest";
/// Defensive cap: the release JSON is a few KB; refuse to buffer an endless body.
const MAX_BODY: usize = 1_000_000;

/// A newer release that the last check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Version without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// Release page to open in the browser.
    pub url: String,
}

/// Outcome of a check, suitable for handing across the thread boundary.
pub enum CheckResult {
    /// A release newer than [`VERSION`] exists.
    Available(UpdateInfo),
    /// This build is current (or newer than anything published).
    UpToDate,
    /// The check could not complete; the string is a short reason for the user.
    Failed(String),
}

/// Ask GitHub for the latest release. Never panics; every failure comes back as
/// [`CheckResult::Failed`]. Blocks on the network — call it off the UI thread.
pub fn check() -> CheckResult {
    match fetch_latest() {
        Ok(body) => assess(&body, VERSION),
        Err(e) => CheckResult::Failed(e),
    }
}

/// Decide what `body` (a GitHub *latest release* payload) means for `current`.
fn assess(body: &str, current: &str) -> CheckResult {
    let Some(tag) = json_string(body, "tag_name") else {
        return CheckResult::Failed("响应中没有版本信息".to_string());
    };
    if is_newer(&tag, current) {
        let url = json_string(body, "html_url").unwrap_or_else(|| FALLBACK_URL.to_string());
        CheckResult::Available(UpdateInfo {
            version: normalize_version(&tag),
            url,
        })
    } else {
        CheckResult::UpToDate
    }
}

// ------------------------------------------------------------- version maths

/// `v0.2.0` / `V1.3` -> `0.2.0` / `1.3`; anything already bare is unchanged.
fn normalize_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Parse `vMAJOR.MINOR.PATCH…` into its numeric components. A pre-release or
/// build suffix (`-beta.1`, `+build`) is ignored, and so is any trailing
/// non-numeric garbage. `None` when there is no leading number at all.
fn parse_version(s: &str) -> Option<Vec<u64>> {
    // Drop any pre-release / build metadata first, so `0.1.0-beta.1` compares as
    // `0.1.0` rather than turning `-beta` into a spurious fourth component.
    let s = normalize_version(s);
    let core = s.split(['-', '+']).next().unwrap_or("");
    let mut out = Vec::new();
    for part in core.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            break;
        }
        out.push(digits.parse().ok()?);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// True when `latest` is a strictly higher version than `current`. Missing
/// components count as zero (`1.2` == `1.2.0`), so the two only differ on a
/// genuine increase. Unparseable input is never treated as newer.
fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(latest), parse_version(current)) else {
        return false;
    };
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

// ------------------------------------------------------------- tiny JSON read

/// Pull the string value of a top-level `"key"` out of a JSON object. Enough for
/// the two flat fields we care about (`tag_name`, `html_url`); deliberately not
/// a general parser. Values here never contain escaped quotes.
fn json_string(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let colon = rest.find(':')?;
    let after = &rest[colon + 1..];
    let open = after.find('"')? + 1;
    let after = &after[open..];
    let close = after.find('"')?;
    Some(after[..close].to_string())
}

// ------------------------------------------------------------- WinHTTP fetch

/// GET the latest-release JSON. Blocking; bounded by [`WinHttpSetTimeouts`].
#[cfg(windows)]
fn fetch_latest() -> Result<String, String> {
    unsafe {
        let agent = to_wide(&format!("PeekGuard/{VERSION}"));
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("无法初始化 WinHTTP".to_string());
        }
        // 5 s to resolve/connect/send, 10 s to receive. An offline or blackholed
        // network fails fast instead of hanging the check.
        WinHttpSetTimeouts(session, 5000, 5000, 5000, 10_000);

        let host = to_wide(REPO_HOST);
        let connect = WinHttpConnect(session, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if connect.is_null() {
            WinHttpCloseHandle(session);
            return Err("无法连接 GitHub".to_string());
        }

        let verb = to_wide("GET");
        let path = to_wide(REPO_LATEST_PATH);
        let request = WinHttpOpenRequest(
            connect,
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            WinHttpCloseHandle(connect);
            WinHttpCloseHandle(session);
            return Err("无法创建请求".to_string());
        }

        // No extra headers: the agent string passed to WinHttpOpen already becomes
        // the User-Agent, which is what the GitHub API requires.
        let sent = WinHttpSendRequest(request, std::ptr::null(), 0, std::ptr::null_mut(), 0, 0, 0);
        if sent == 0 || WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
            WinHttpCloseHandle(request);
            WinHttpCloseHandle(connect);
            WinHttpCloseHandle(session);
            return Err("请求 GitHub 失败".to_string());
        }

        let status = query_status(request);
        let body = read_body(request);
        WinHttpCloseHandle(request);
        WinHttpCloseHandle(connect);
        WinHttpCloseHandle(session);

        match (status, body) {
            (Some(200), Ok(b)) => Ok(b),
            (Some(s), _) => Err(format!("GitHub 返回 HTTP {s}")),
            (None, _) => Err("读取响应状态失败".to_string()),
        }
    }
}

// ------------------------------------------------------------- curl fetch (Linux)

/// GET the latest-release JSON through the system `curl`. Blocking; bounded by
/// `--max-time`. A missing curl is reported as a plain failure, so startup
/// checks stay quiet and manual checks explain themselves.
#[cfg(target_os = "linux")]
fn fetch_latest() -> Result<String, String> {
    let url = format!("https://{REPO_HOST}{REPO_LATEST_PATH}");
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "10",
            "-A",
            &format!("PeekGuard/{VERSION}"),
            &url,
        ])
        .output()
        .map_err(|e| format!("无法运行 curl（{e}）"))?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(c) => format!("curl 请求失败（退出码 {c}）"),
            None => "curl 请求失败".to_string(),
        });
    }
    if out.stdout.len() > MAX_BODY {
        return Err("响应过大".to_string());
    }
    String::from_utf8(out.stdout).map_err(|_| "响应不是有效的 UTF-8".to_string())
}

#[cfg(windows)]
unsafe fn query_status(request: HINTERNET) -> Option<u32> {
    let mut code: u32 = 0;
    let mut len: u32 = std::mem::size_of::<u32>() as u32;
    let ok = WinHttpQueryHeaders(
        request,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        std::ptr::null(),
        &mut code as *mut u32 as *mut c_void,
        &mut len,
        std::ptr::null_mut(),
    );
    if ok != 0 {
        Some(code)
    } else {
        None
    }
}

#[cfg(windows)]
unsafe fn read_body(request: HINTERNET) -> Result<String, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut read: u32 = 0;
        let ok = WinHttpReadData(
            request,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut read,
        );
        if ok == 0 {
            return Err("读取响应体失败".to_string());
        }
        if read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
        if out.len() > MAX_BODY {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.1.0", "0.0.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn missing_components_are_zero() {
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(is_newer("1.2.1", "1.2"));
        assert!(is_newer("1.3", "1.2.9"));
    }

    #[test]
    fn prerelease_suffix_is_ignored() {
        // A `-beta` on the same numbers is not "newer"; the numeric core decides.
        assert!(!is_newer("v0.1.0-beta.1", "0.1.0"));
        assert!(is_newer("v0.2.0-rc.1", "0.1.0"));
    }

    #[test]
    fn junk_versions_are_never_newer() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("0.1.0", "garbage"));
    }

    #[test]
    fn normalize_strips_v_prefix() {
        assert_eq!(normalize_version("v0.2.0"), "0.2.0");
        assert_eq!(normalize_version("V1.0"), "1.0");
        assert_eq!(normalize_version("0.1.0"), "0.1.0");
    }

    #[test]
    fn extracts_flat_json_fields() {
        let body = r#"{"url":"x","tag_name":"v0.2.0","html_url":"https://example/rel"}"#;
        assert_eq!(json_string(body, "tag_name").as_deref(), Some("v0.2.0"));
        assert_eq!(
            json_string(body, "html_url").as_deref(),
            Some("https://example/rel")
        );
        assert_eq!(json_string(body, "missing"), None);
    }

    #[test]
    fn assess_reports_a_newer_release() {
        let body = r#"{"tag_name":"v9.9.9","html_url":"https://example/new"}"#;
        match assess(body, "0.1.0") {
            CheckResult::Available(info) => {
                assert_eq!(info.version, "9.9.9");
                assert_eq!(info.url, "https://example/new");
            }
            _ => panic!("expected Available"),
        }
        match assess(body, "9.9.9") {
            CheckResult::UpToDate => {}
            _ => panic!("expected UpToDate"),
        }
    }

    #[test]
    fn assess_falls_back_to_known_url() {
        let body = r#"{"tag_name":"v9.9.9"}"#;
        match assess(body, "0.1.0") {
            CheckResult::Available(info) => assert_eq!(info.url, FALLBACK_URL),
            _ => panic!("expected Available"),
        }
    }
}
