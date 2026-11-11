//! The audio fingerprint an index is cached under.
//!
//! Downmix to mono 16 kHz first: it is what ASR backends want anyway, it makes
//! the hash independent of the container, and it keeps the fingerprint cheap to
//! compute on a long recording.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::ffmpeg::Invocation;
use crate::paths::arg;
use crate::project::Store;
use crate::time::Time;

pub const SAMPLE_RATE: u64 = 16_000;
const BYTES_PER_SEC: u64 = SAMPLE_RATE * 2; // mono s16le

pub struct Fingerprint {
    pub sha256: String,
    pub duration: Time,
    pub wav: PathBuf,
}

/// Extracts (once) and hashes the audio of `source`.
pub fn fingerprint(store: &Store, source: &Path) -> Result<Fingerprint> {
    if !source.exists() {
        bail!("media_missing: {} does not exist", source.display());
    }
    let stat = crate::cache::stat_token(source);
    let key = crate::cache::hash(&[stat, "audio16k".into()]);
    let wav = store.dir().join(".splice").join("audio").join(format!("{key}.wav"));

    if !wav.exists() {
        if let Some(parent) = wav.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut inv = Invocation::new();
        inv.input(source);
        inv.push("-vn")
            .push("-ac")
            .push("1")
            .push("-ar")
            .push(SAMPLE_RATE.to_string())
            .push("-c:a")
            .push("pcm_s16le")
            .push(arg(&wav));
        crate::ffmpeg::run(&inv, std::time::Duration::from_secs(600))
            .with_context(|| format!("extracting audio from {}", source.display()))?;
    }

    let bytes = std::fs::metadata(&wav).context("reading extracted audio")?.len();
    if bytes < 44 {
        bail!("extracted audio is empty: {}", wav.display());
    }
    let duration = Time::from_micros(((bytes.saturating_sub(44)) * 1_000_000 / BYTES_PER_SEC) as i64);
    Ok(Fingerprint {
        sha256: sha256_file(&wav)?,
        duration,
        wav,
    })
}

/// The bytes to upload: 16 kHz mono mp3 at 64k. Ten minutes is about 4.8 MB,
/// which stays under Gemini's 20 MB inline limit and is small enough that an
/// agent's whole session can be transcribed without thinking about bandwidth.
///
/// Separate from `fingerprint` (which extracts lossless wav for hashing) because
/// the hash must be of the audio, not of a compressed rendering of it.
pub struct AsrAudio {
    pub path: PathBuf,
    pub sha256: String,
}

pub fn asr_audio(store: &Store, source: &Path) -> Result<AsrAudio> {
    if !source.exists() {
        bail!("media_missing: {} does not exist", source.display());
    }
    let stat = crate::cache::stat_token(source);
    let key = crate::cache::hash(&[stat, "asr16k64".into()]);
    let path = store
        .dir()
        .join(".splice")
        .join("asr")
        .join(format!("{key}.mp3"));
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut inv = Invocation::new();
        inv.input(source);
        inv.push("-vn")
            .push("-ac")
            .push("1")
            .push("-ar")
            .push(SAMPLE_RATE.to_string())
            .push("-c:a")
            .push("libmp3lame")
            .push("-b:a")
            .push("64k")
            .push(arg(&path));
        crate::ffmpeg::run(&inv, std::time::Duration::from_secs(600))
            .with_context(|| format!("extracting audio from {}", source.display()))?;
    }
    Ok(AsrAudio {
        sha256: sha256_file(&path)?,
        path,
    })
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
