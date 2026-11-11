//! Where keys and model ids live, and which source wins.
//!
//! A plain JSON file per user, never in the project: `splice.json` gets committed
//! and pasted into transcripts, so a key there is a key in public. There is no
//! `--api-key` flag either — argv shows up in `ps`, in shell history and in the
//! JSON an agent echoes back.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;

/// Providers that can hold a key or a model. `whisper` is an external binary, so
/// it has a model name and a path but never a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    Deepgram,
    Whisper,
}

impl Provider {
    pub fn all() -> [Provider; 3] {
        [Provider::Gemini, Provider::Deepgram, Provider::Whisper]
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::Deepgram => "deepgram",
            Provider::Whisper => "whisper",
        }
    }

    pub fn parse(raw: &str) -> Option<Provider> {
        Provider::all().into_iter().find(|p| p.id() == raw)
    }

    /// Environment variable that supplies this provider's key. CI mounts tokens
    /// as env vars; the file is for machines that have one.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Provider::Gemini => Some("GEMINI_API_KEY"),
            Provider::Deepgram => Some("DEEPGRAM_API_KEY"),
            Provider::Whisper => None,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini-3.5-transcribe",
            Provider::Deepgram => "nova-3",
            Provider::Whisper => "small",
        }
    }
}

/// A secret that will not print itself. The only way out is `expose`, so any
/// place a key reaches a string is a visible, greppable call site.
#[derive(Clone, Deserialize)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into().trim().to_string())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// First 8 hex of the SHA-256. Answers "did the env win" and "same key as
    /// yesterday", and is useless for reconstructing the key.
    pub fn fingerprint(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.0.as_bytes());
        h.finalize().iter().take(4).map(|b| format!("{b:02x}")).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Serializes as a plain string because it has to be written to the file. The
/// redaction lives in `Debug` and in the absence of `Display`, which are the two
/// ways a value normally escapes into a log or a JSON payload.
impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(<redacted {})>", self.fingerprint())
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<Secret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, ProviderConfig>,
}

fn one() -> u32 {
    VERSION
}

impl Default for Config {
    fn default() -> Config {
        Config {
            version: VERSION,
            providers: Default::default(),
        }
    }
}

impl Config {
    pub fn provider(&self, p: Provider) -> ProviderConfig {
        self.providers.get(p.id()).cloned().unwrap_or_default()
    }

    pub fn provider_mut(&mut self, p: Provider) -> &mut ProviderConfig {
        self.providers.entry(p.id().to_string()).or_default()
    }
}

/// `$SPLICE_CONFIG`, else the platform config dir, else an error. Never the
/// working directory: precedence that depends on `cd` is a bug waiting to happen.
pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("SPLICE_CONFIG") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let dir = if cfg!(windows) {
        std::env::var("APPDATA").map(PathBuf::from).context(
            "no APPDATA; set SPLICE_CONFIG to choose a config file explicitly",
        )?
    } else if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if x.trim().is_empty() {
            home()?.join(".config")
        } else {
            PathBuf::from(x)
        }
    } else {
        home()?.join(".config")
    };
    Ok(dir.join("splice").join("config.json"))
}

fn home() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("no HOME; set SPLICE_CONFIG to choose a config file explicitly")
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    load_at(&path)
}

pub fn load_at(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let cfg: Config = serde_json::from_str(&raw)
                .with_context(|| format!("{} is not a valid config", path.display()))?;
            if cfg.version != VERSION {
                bail!("config version {} is not supported", cfg.version);
            }
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config {
            version: VERSION,
            providers: Default::default(),
        }),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save(cfg: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    save_at(cfg, &path)?;
    Ok(path)
}

pub fn save_at(cfg: &Config, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        restrict(dir, 0o700)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(cfg)? + "\n")
        .with_context(|| format!("writing {}", tmp.display()))?;
    restrict(&tmp, 0o600)?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<()> {
    // Windows inherits the profile's ACL; chmod is not the mechanism there.
    Ok(())
}

/// True when someone else can read the file. Reported, never silently fixed.
#[cfg(unix)]
pub fn loose_perms(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) => m.permissions().mode() & 0o077 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub fn loose_perms(_path: &Path) -> bool {
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A flag on the command being run.
    Flag,
    /// An environment variable, by name.
    Env,
    /// The config file.
    File,
    /// The pinned default.
    Default,
}

#[derive(Clone, Debug)]
pub struct Resolved {
    pub key: Option<Secret>,
    pub key_source: Option<Origin>,
    pub model: String,
    pub model_source: Origin,
}

impl Resolved {
    /// Environment wins for keys: a container that mounts a token should not have
    /// to write a file too.
    pub fn get(
        p: Provider,
        cfg: &Config,
        flag_model: Option<&str>,
        env_lookup: impl Fn(&str) -> Option<String>,
    ) -> Resolved {
        let pc = cfg.provider(p);
        let (key, key_source) = match p.key_env().and_then(&env_lookup) {
            Some(v) if !v.trim().is_empty() => (Some(Secret::new(v)), Some(Origin::Env)),
            _ => match pc.api_key.filter(|k| !k.is_empty()) {
                Some(k) => (Some(k), Some(Origin::File)),
                None => (None, None),
            },
        };
        let (model, model_source) = match flag_model {
            Some(m) => (m.to_string(), Origin::Flag),
            None => match pc.model.filter(|m| !m.trim().is_empty()) {
                Some(m) => (m, Origin::File),
                None => (p.default_model().to_string(), Origin::Default),
            },
        };
        Resolved {
            key,
            key_source,
            model,
            model_source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(p: Provider, key: Option<&str>, model: Option<&str>) -> Config {
        let mut c = Config::default();
        let pc = c.provider_mut(p);
        if let Some(k) = key {
            pc.api_key = Some(Secret::new(k));
        }
        if let Some(m) = model {
            pc.model = Some(m.to_string());
        }
        c
    }

    #[test]
    fn environment_beats_the_file() {
        let cfg = cfg_with(Provider::Gemini, Some("file-key"), None);
        let r = Resolved::get(Provider::Gemini, &cfg, None, |v| {
            (v == "GEMINI_API_KEY").then(|| "env-key".to_string())
        });
        assert_eq!(r.key.as_ref().unwrap().expose(), "env-key");
        assert_eq!(r.key_source, Some(Origin::Env));

        let r = Resolved::get(Provider::Gemini, &cfg, None, |_| None);
        assert_eq!(r.key.as_ref().unwrap().expose(), "file-key");
        assert_eq!(r.key_source, Some(Origin::File));
    }

    #[test]
    fn an_empty_environment_variable_does_not_win() {
        let cfg = cfg_with(Provider::Deepgram, Some("file-key"), None);
        let r = Resolved::get(Provider::Deepgram, &cfg, None, |_| Some("   ".into()));
        assert_eq!(r.key_source, Some(Origin::File));
    }

    #[test]
    fn a_flag_beats_a_configured_model_which_beats_the_default() {
        let cfg = cfg_with(Provider::Deepgram, None, Some("nova-4"));
        let r = Resolved::get(Provider::Deepgram, &cfg, Some("nova-5"), |_| None);
        assert_eq!((r.model.as_str(), r.model_source), ("nova-5", Origin::Flag));
        let r = Resolved::get(Provider::Deepgram, &cfg, None, |_| None);
        assert_eq!((r.model.as_str(), r.model_source), ("nova-4", Origin::File));
        let r = Resolved::get(Provider::Whisper, &Config::default(), None, |_| None);
        assert_eq!((r.model.as_str(), r.model_source), ("small", Origin::Default));
    }

    #[test]
    fn whisper_has_no_key_variable() {
        assert!(Provider::Whisper.key_env().is_none());
        let cfg = cfg_with(Provider::Whisper, Some("ignored"), None);
        let r = Resolved::get(Provider::Whisper, &cfg, None, |v| Some(format!("{v}-value")));
        // no env var to consult, so the file's key is what is reported
        assert_eq!(r.key_source, Some(Origin::File));
    }

    #[test]
    fn a_secret_does_not_render_itself() {
        let s = Secret::new("sk-super-secret-value");
        assert!(!format!("{s:?}").contains("secret-value"));
        assert_eq!(s.fingerprint().len(), 8);
        assert_ne!(s.fingerprint(), Secret::new("other").fingerprint());
        assert_eq!(s.fingerprint(), Secret::new("sk-super-secret-value").fingerprint());
    }

    #[test]
    fn round_trips_through_a_file_with_tight_permissions() {
        let dir = std::env::temp_dir().join(format!("splice-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("splice").join("config.json");
        let mut cfg = Config::default();
        cfg.provider_mut(Provider::Gemini).api_key = Some(Secret::new("k1"));
        cfg.provider_mut(Provider::Gemini).model = Some("gemini-3.5-transcribe".into());
        save_at(&cfg, &path).unwrap();

        let back = load_at(&path).unwrap();
        assert_eq!(
            back.provider(Provider::Gemini).api_key.unwrap().expose(),
            "k1"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "config must not be world-readable");
            assert!(!loose_perms(&path));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_an_empty_config_not_an_error() {
        let path = std::env::temp_dir().join("splice-cfg-does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        let cfg = load_at(&path).unwrap();
        assert!(cfg.providers.is_empty());
    }
}
