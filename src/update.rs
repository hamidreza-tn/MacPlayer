//! Update checker: resolves the latest published release tag from GitHub and
//! compares it against the running version. Uses `curl` (present on every
//! macOS/Linux box we target) so no HTTP client dependency is added.

use std::process::{Command, Stdio};
const RELEASES_API: &str = "https://api.github.com/repos/hamidreza-tn/MacPlayer/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/hamidreza-tn/MacPlayer/releases/latest";

/// GitHub releases page, opened by the GUI and printed by the CLI.
pub fn releases_page() -> &'static str {
    RELEASES_PAGE
}

/// Fetch the latest release tag ("v1.9.4"), if reachable within the timeout.
pub fn fetch_latest_tag() -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-s",
            "-m",
            "6",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASES_API,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    extract_tag(&text)
}

/// Pull the `tag_name` field out of the release JSON via a real parser, so
/// the key can never be confused with a lookalike inside another string.
fn extract_tag(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = value.get("tag_name")?.as_str()?;
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Parse "v1.9.4" / "1.9.4" / "v2.0.0-rc1" into a comparable triple
/// (pre-release and build suffixes are stripped, so an RC of a newer
/// release still counts as newer).
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let digits = tag.trim().trim_start_matches(['v', 'V']);
    let digits = digits.split(['-', '+']).next()?;
    let mut parts = digits.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// `true` when `latest` is strictly newer than the running version.
pub fn is_newer(latest: &str) -> bool {
    let Some(latest) = parse_version(latest) else {
        return false;
    };
    let Some(current) = parse_version(env!("CARGO_PKG_VERSION")) else {
        return false;
    };
    latest > current
}

/// Outcome of an update check: `fetch_latest_tag` failing (offline, no curl,
/// timeout) is reported distinctly from "no update".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// A newer release tag is published.
    Available(String),
    /// The latest tag is not newer than the running version.
    UpToDate,
    /// The check could not run (offline, no curl, timeout).
    Unknown,
}

/// One-shot check, bounded by curl's own timeout.
pub fn check_for_update() -> UpdateStatus {
    match fetch_latest_tag() {
        None => UpdateStatus::Unknown,
        Some(tag) if is_newer(&tag) => UpdateStatus::Available(tag),
        Some(_) => UpdateStatus::UpToDate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tag_handles_real_json() {
        let json = r#"{"url":"https://…","tag_name": "v1.9.4" ,"name":"MacPlayer"}"#;
        assert_eq!(extract_tag(json), Some("v1.9.4".into()));
        assert_eq!(extract_tag("{}"), None);
    }

    #[test]
    fn version_compare_is_semantic() {
        assert!(parse_version("v1.9.4").unwrap() > parse_version("1.9.3").unwrap());
        assert!(is_newer("v2.0.0"));
        assert!(!is_newer("v1.9.4"));
        assert!(!is_newer("v1.9.3"));
        assert!(!is_newer("garbage"));
        assert!(!is_newer(""));
    }
}
