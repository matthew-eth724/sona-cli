use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

/// The bytes at `url`: `file://` paths are read from disk, http(s) go through ureq.
/// The whole body is buffered, because these assets are 20-30 MB gzipped and are
/// hashed before anything is written into place.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).with_context(|| format!("reading {path}"));
    }
    let mut body = Vec::new();
    ureq::get(url)
        .call()
        .with_context(|| format!("fetching {url}"))?
        .into_body()
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| anyhow!("reading the body of {url}: {e}"))?;
    Ok(body)
}

/// A platform the installer knows how to pin: id, OS, arch.
pub struct Platform {
    pub id: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

/// (id, ffmpeg sha256, ffprobe sha256, ffmpeg asset, ffprobe asset).
pub const PINS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "linux-x64",
        "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
        "4f231a1960d83e403d08f7971e271707bec278a9ae18e21b8b5b03186668450d",
        "ffmpeg-linux-x64.gz",
        "ffprobe-linux-x64.gz",
    ),
    (
        "linux-arm64",
        "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce",
        "d17ae9b4c297d48e2521ba14e417bb0537c6ff77c584cdbcd6bb0d8d0307a2e8",
        "ffmpeg-linux-arm64.gz",
        "ffprobe-linux-arm64.gz",
    ),
    (
        "darwin-arm64",
        "a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584",
        "bb2db6f5d8cef919da12fbf592119a987202a8c060a886f3cab091f9cab90b64",
        "ffmpeg-darwin-arm64.gz",
        "ffprobe-darwin-arm64.gz",
    ),
    (
        "darwin-x64",
        "ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894",
        "fa3add0ce901f7241abe0dfc0155d958fc834aca3f8ce61f87cc712ae669c1e0",
        "ffmpeg-darwin-x64.gz",
        "ffprobe-darwin-x64.gz",
    ),
    (
        "win32-x64",
        "04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00",
        "3a7e2dc003dc2cd1472827e4c7c4f056ae1ae0ae7c5bbc580c99b49827351ba4",
        "ffmpeg-win32-x64.gz",
        "ffprobe-win32-x64.gz",
    ),
];

/// Where the pinned `ffmpeg-<platform>.gz` / `ffprobe-<platform>.gz` assets are
/// served from; `install` fetches `{base}/{asset}`. Mirrors the pin set used by
/// `tools/ci/fetch-ffmpeg.sh` (eugeneware/ffmpeg-static, tag b6.1.1).
pub const BASE_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1";

/// The pinned platform for this build (`std::env::consts::{OS, ARCH}`).
pub fn platform() -> Option<Platform> {
    let (id, os, arch) = match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "linux", "x86_64"),
        ("linux", "aarch64") => ("linux-arm64", "linux", "aarch64"),
        ("macos", "aarch64") => ("darwin-arm64", "macos", "aarch64"),
        ("macos", "x86_64") => ("darwin-x64", "macos", "x86_64"),
        ("windows", "x86_64") => ("win32-x64", "windows", "x86_64"),
        _ => return None,
    };
    PINS.iter().find(|(p, _, _, _, _)| *p == id)?;
    Some(Platform { id, os, arch })
}

/// The installed file name for a tool on this platform.
pub fn bin_name(tool: &str, p: &Platform) -> String {
    if p.os == "windows" { format!("{tool}.exe") } else { tool.to_string() }
}

/// Gunzip a whole buffer. The release assets are gzip, and Windows has no `gunzip`
/// we can rely on, so decompression happens in-process.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .read_to_end(&mut out)
        .map_err(|e| anyhow!("not a gzip body: {e}"))?;
    Ok(out)
}

/// Lowercase hex sha256 of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// If both installed files exist and match the pins for `p`, return their hashes.
/// `None` means "not installed" or "does not match"; either way, install again.
pub fn installed_hashes(
    dir: &Path,
    p: &Platform,
    ffmpeg_pin: &str,
    ffprobe_pin: &str,
) -> Option<(String, String)> {
    let ffmpeg = std::fs::read(dir.join(bin_name("ffmpeg", p))).ok()?;
    let ffprobe = std::fs::read(dir.join(bin_name("ffprobe", p))).ok()?;
    let ffmpeg_hash = sha256_hex(&ffmpeg);
    let ffprobe_hash = sha256_hex(&ffprobe);
    if ffmpeg_hash == *ffmpeg_pin && ffprobe_hash == *ffprobe_pin {
        Some((ffmpeg_hash, ffprobe_hash))
    } else {
        None
    }
}

/// Pure form of `cache_dir`, so the per-OS conventions can be tested without
/// mutating the process environment.
fn cache_dir_from(
    os: &str,
    xdg: Option<&str>,
    home: Option<&str>,
    local_appdata: Option<&str>,
) -> Option<PathBuf> {
    match os {
        "linux" => {
            let base = match xdg {
                Some(d) => PathBuf::from(d),
                None => PathBuf::from(home?).join(".cache"),
            };
            Some(base.join("splice/ffmpeg"))
        }
        "macos" => Some(PathBuf::from(home?).join("Library/Caches/splice/ffmpeg")),
        "windows" => local_appdata
            .or_else(|| home)
            .map(|base| PathBuf::from(base).join("splice/ffmpeg")),
        _ => None,
    }
}

/// Per-OS cache directory: `splice/ffmpeg` under the OS cache root.
pub fn cache_dir() -> Option<PathBuf> {
    let xdg = env::var("XDG_CACHE_HOME").ok();
    let home = env::var("HOME").ok();
    let local_appdata = env::var_os("LOCALAPPDATA")
        .map(|s| s.to_string_lossy().into_owned());
    cache_dir_from(
        env::consts::OS,
        xdg.as_deref(),
        home.as_deref(),
        local_appdata.as_deref(),
    )
}

#[derive(Debug)]
pub struct Outcome {
    pub installed: bool,
    pub already_installed: bool,
    pub dir: PathBuf,
    pub platform: String,
    pub ffmpeg_sha256: String,
    pub ffprobe_sha256: String,
}

/// Removes the staging directory on drop, so every exit after the stage is
/// created cleans it up — both the error paths and the success path.
struct StageDir {
    path: PathBuf,
}

impl Drop for StageDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn install(
    p: &Platform,
    dir: &Path,
    ffmpeg_src: &str,
    ffprobe_src: &str,
    ffmpeg_pin: &str,
    ffprobe_pin: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<Outcome> {
    if let Some((ffmpeg_hash, ffprobe_hash)) =
        installed_hashes(dir, p, ffmpeg_pin, ffprobe_pin)
    {
        return Ok(Outcome {
            installed: false,
            already_installed: true,
            dir: dir.to_path_buf(),
            platform: p.id.to_string(),
            ffmpeg_sha256: ffmpeg_hash,
            ffprobe_sha256: ffprobe_hash,
        });
    }

    let stage = dir.join(format!(".tmp-{}", std::process::id()));
    std::fs::create_dir_all(&stage)?;
    let _guard = StageDir { path: stage.clone() };

    for (name, src, pin) in [
        ("ffmpeg", ffmpeg_src, ffmpeg_pin),
        ("ffprobe", ffprobe_src, ffprobe_pin),
    ] {
        on_progress(&format!("{name}: downloading"));
        let gz = fetch_bytes(src)?;
        on_progress(&format!("{name}: {:.1} MB", gz.len() as f64 / 1_048_576.0));
        let bytes = gunzip(&gz)?;
        let got = sha256_hex(&bytes);
        if got != *pin {
            bail!("{name} sha256 mismatch: expected {pin}, got {got}");
        }
        std::fs::write(stage.join(bin_name(name, p)), &bytes)?;
    }

    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for tool in ["ffmpeg", "ffprobe"] {
            std::fs::set_permissions(
                stage.join(bin_name(tool, p)),
                std::fs::Permissions::from_mode(0o755),
            )?;
        }
    }
    std::fs::rename(stage.join(bin_name("ffmpeg", p)), dir.join(bin_name("ffmpeg", p)))?;
    std::fs::rename(stage.join(bin_name("ffprobe", p)), dir.join(bin_name("ffprobe", p)))?;

    Ok(Outcome {
        installed: true,
        already_installed: false,
        dir: dir.to_path_buf(),
        platform: p.id.to_string(),
        ffmpeg_sha256: ffmpeg_pin.to_string(),
        ffprobe_sha256: ffprobe_pin.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_has_64_hex_chars() {
        for (_, ffmpeg, ffprobe, _, _) in PINS {
            for hash in [ffmpeg, ffprobe] {
                assert_eq!(hash.len(), 64, "{hash} is not 64 chars");
                assert!(
                    hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                    "{hash} is not lowercase hex"
                );
            }
        }
    }

    #[test]
    fn no_two_tools_share_a_hash() {
        let mut hashes: Vec<&&str> = Vec::new();
        for (_, ffmpeg, ffprobe, _, _) in PINS {
            hashes.push(ffmpeg);
            hashes.push(ffprobe);
        }
        let mut sorted = hashes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), hashes.len(), "duplicate hash in PINS");
    }

    #[test]
    fn platform_ids_are_pinned() {
        let first = platform();
        let second = platform();
        assert_eq!(
            first.as_ref().map(|p| p.id),
            second.as_ref().map(|p| p.id),
            "platform() is not deterministic"
        );
        if let Some(p) = first {
            assert!(
                PINS.iter().any(|(id, _, _, _, _)| *id == p.id),
                "{} is not pinned",
                p.id
            );
        }
    }

    #[test]
    fn fetch_bytes_reads_a_file_url() {
        let path = std::env::temp_dir().join(format!(
            "ffmpeg_install_fetch_bytes_test-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"hello").unwrap();
        let got = fetch_bytes(&format!("file://{}", path.display())).unwrap();
        assert_eq!(got, b"hello");
        assert!(
            fetch_bytes("http://127.0.0.1:1/x").is_err(),
            "http to a dead address should fail"
        );
    }

    #[test]
    fn windows_names_end_in_exe() {
        let win = Platform {
            id: "win32-x64",
            os: "windows",
            arch: "x64",
        };
        assert_eq!(bin_name("ffmpeg", &win), "ffmpeg.exe");
        let lin = Platform {
            id: "linux-x64",
            os: "linux",
            arch: "x64",
        };
        assert_eq!(bin_name("ffmpeg", &lin), "ffmpeg");
    }

    #[test]
    fn gunzip_error_is_not_an_io_error() {
        let err = gunzip(b"not gzip").unwrap_err();
        assert!(err.downcast_ref::<std::io::Error>().is_none(), "{err}");
    }

    #[test]
    fn gunzip_round_trips() {
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"hello").unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(gunzip(&compressed).unwrap(), b"hello");
        assert!(gunzip(b"not gzip").is_err());
    }

    #[test]
    fn sha256_hex_matches_a_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn installed_hashes_is_none_for_a_missing_dir() {
        let gone = std::env::temp_dir().join(format!(
            "ffmpeg-install-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&gone);
        let p = Platform {
            id: "linux-x64",
            os: "linux",
            arch: "x86_64",
        };
        assert_eq!(
            installed_hashes(&gone, &p, &"0".repeat(64), &"0".repeat(64)),
            None
        );
    }

    fn gz(payload: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(payload).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn linux_cache_follows_xdg_then_home() {
        assert_eq!(
            cache_dir_from("linux", Some("/x"), None, None),
            Some(PathBuf::from("/x/splice/ffmpeg"))
        );
        assert_eq!(
            cache_dir_from("linux", None, Some("/h"), None),
            Some(PathBuf::from("/h/.cache/splice/ffmpeg"))
        );
        assert_eq!(cache_dir_from("linux", None, None, None), None);
    }

    #[test]
    fn macos_and_windows_cache_paths() {
        assert_eq!(
            cache_dir_from("macos", None, Some("/h"), None),
            Some(PathBuf::from("/h/Library/Caches/splice/ffmpeg"))
        );
        assert_eq!(
            cache_dir_from("windows", None, None, Some("C:/L")),
            Some(PathBuf::from("C:/L/splice/ffmpeg"))
        );
        assert_eq!(cache_dir_from("windows", None, None, None), None);
    }

    fn plat() -> Platform {
        Platform { id: "linux-x64", os: "linux", arch: "x64" }
    }

    fn fixture_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("splice-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn installs_both_files_and_reports_hashes() {
        let f = fixture_dir("ok");
        let a = f.join("a.gz");
        let b = f.join("b.gz");
        std::fs::write(&a, gz(b"FFMPEG")).unwrap();
        std::fs::write(&b, gz(b"FFPROBE")).unwrap();
        let dir = f.join("out");
        let out = install(
            &plat(),
            &dir,
            &format!("file://{}", a.display()),
            &format!("file://{}", b.display()),
            &sha256_hex(b"FFMPEG"),
            &sha256_hex(b"FFPROBE"),
            &mut |_| {},
        )
        .unwrap();
        assert!(out.installed && !out.already_installed);
        assert_eq!(std::fs::read(dir.join("ffmpeg")).unwrap(), b"FFMPEG");
        let _ = std::fs::remove_dir_all(&f);
    }

    #[test]
    fn a_second_run_makes_no_request() {
        let f = fixture_dir("warm");
        let a = f.join("a.gz");
        let b = f.join("b.gz");
        std::fs::write(&a, gz(b"FFMPEG")).unwrap();
        std::fs::write(&b, gz(b"FFPROBE")).unwrap();
        let dir = f.join("out");
        let pins = (sha256_hex(b"FFMPEG"), sha256_hex(b"FFPROBE"));
        let _ = install(
            &plat(), &dir,
            &format!("file://{}", a.display()),
            &format!("file://{}", b.display()),
            &pins.0, &pins.1, &mut |_| {},
        ).unwrap();
        let mut lines = Vec::new();
        let out = install(
            &plat(), &dir,
            "file:///nonexistent-a", "file:///nonexistent-b",
            &pins.0, &pins.1,
            &mut |l| lines.push(l.to_string()),
        ).unwrap();
        assert!(out.already_installed);
        assert!(!lines.iter().any(|l| l.contains("downloading")));
        let _ = std::fs::remove_dir_all(&f);
    }

    #[test]
    fn hash_mismatch_leaves_the_previous_install_untouched() {
        let f = fixture_dir("mismatch");
        let a = f.join("a.gz");
        let b = f.join("b.gz");
        std::fs::write(&a, gz(b"FFMPEG")).unwrap();
        std::fs::write(&b, gz(b"FFPROBE")).unwrap();
        let dir = f.join("out");
        let pins = (sha256_hex(b"FFMPEG"), sha256_hex(b"FFPROBE"));
        let _ = install(
            &plat(), &dir,
            &format!("file://{}", a.display()),
            &format!("file://{}", b.display()),
            &pins.0, &pins.1, &mut |_| {},
        ).unwrap();

        // Pins that do NOT match what is installed, so `installed_hashes` cannot
        // short-circuit and the fetch really happens — with a payload that does not
        // match the pin either.
        let c = f.join("c.gz");
        std::fs::write(&c, gz(b"OTHER")).unwrap();
        let wrong = sha256_hex(b"SOMETHING-ELSE");
        let err = install(
            &plat(), &dir,
            &format!("file://{}", c.display()),
            &format!("file://{}", b.display()),
            &wrong, &pins.1, &mut |_| {},
        ).unwrap_err();
        assert!(
            format!("{err}").contains("sha256 mismatch"),
            "expected a mismatch error, got {err}"
        );
        assert_eq!(std::fs::read(dir.join("ffmpeg")).unwrap(), b"FFMPEG");
        assert_eq!(std::fs::read(dir.join("ffprobe")).unwrap(), b"FFPROBE");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "staging directory was left behind");
        let _ = std::fs::remove_dir_all(&f);
    }
}

