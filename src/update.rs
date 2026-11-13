//! `splice update`: ask the registry what `latest` is, and let npm do the install.
//!
//! The binary is not replaced by this process. On Windows a running executable
//! cannot be overwritten, and npm already knows where the per-platform package
//! lives, which optional dependency resolved, and how the skill's postinstall runs.
//! Reimplementing that would be a second installer with a worse failure mode.
use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};

/// One published version as ordered numbers. Trailing pre-release text is ignored,
/// because comparing `0.1.6` with `0.1.6-beta` is not a decision this command makes.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let core = text.trim().trim_start_matches('v');
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch: String = parts
            .next()
            .unwrap_or("0")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        Some(Version(major, minor, patch.parse().ok()?))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `dist-tags.latest` from the npm registry document.
pub fn latest_from_registry(document: &[u8]) -> Result<String> {
    let json: serde_json::Value = serde_json::from_slice(document)
        .context("the registry did not answer with JSON")?;
    let latest = json
        .get("dist-tags")
        .and_then(|t| t.get("latest"))
        .and_then(|v| v.as_str())
        .context("the registry document has no dist-tags.latest")?;
    Ok(latest.to_string())
}

/// The package document. `SPLICE_REGISTRY` exists so a test can point at a file.
pub fn registry_url() -> String {
    std::env::var("SPLICE_REGISTRY")
        .unwrap_or_else(|_| "https://registry.npmjs.org/splice-cli".to_string())
}

/// What asked-for install command this platform needs. npm ships as `npm.cmd` on
/// Windows, and `Command` will not find it by the bare name.
pub fn npm_program() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

pub fn npm_available() -> bool {
    Command::new(npm_program())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run the install, forwarding npm's own output to stderr: stdout is this program's
/// single JSON object and must not be polluted by a package manager.
pub fn run_npm_install() -> Result<()> {
    if !npm_available() {
        bail!("update_no_npm: npm is not on PATH");
    }
    let output = Command::new(npm_program())
        .args(["install", "-g", "splice-cli@latest"])
        .output()
        .context("running npm")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        eprintln!("{}", stdout.trim_end());
    }
    if !output.status.success() {
        bail!("update_failed: npm exited {}: {}", output.status, stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_by_number_not_by_text() {
        // "0.10.0" must beat "0.9.0"; string comparison would say otherwise.
        assert!(Version::parse("0.10.0") > Version::parse("0.9.0"));
        assert!(Version::parse("0.1.10") > Version::parse("0.1.9"));
        assert_eq!(Version::parse("v0.1.6"), Version::parse("0.1.6"));
        assert_eq!(Version::parse("0.1"), Version::parse("0.1.0"));
    }

    #[test]
    fn junk_is_not_a_version() {
        assert!(Version::parse("latest").is_none());
        assert!(Version::parse("").is_none());
    }

    #[test]
    fn latest_comes_out_of_the_registry_document() {
        let doc = br#"{"dist-tags":{"latest":"0.1.6","next":"0.1.7"},"versions":{}}"#;
        assert_eq!(latest_from_registry(doc).unwrap(), "0.1.6");
    }

    #[test]
    fn a_document_without_a_latest_tag_is_an_error() {
        let doc = br#"{"dist-tags":{"next":"0.1.7"}}"#;
        assert!(latest_from_registry(doc).is_err());
        assert!(latest_from_registry(b"not json").is_err());
    }
}
