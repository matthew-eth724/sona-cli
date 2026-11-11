//! Calling Gemini Transcribe and Deepgram nova-3.
//!
//! Both answers are handed to `transcript::parse_value`, which already
//! understands the two wire formats — so nothing downstream (cues, the index,
//! word-range cuts, ducking) knows or cares which provider produced the words.
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::time::Duration;

use crate::config::{Config, Provider, Resolved, Secret};
use crate::time::Time;
use crate::transcript::Parsed;

/// ASR requests are slow and the audio is already small. Long enough to wait for
/// a 10-minute upload, short enough that a hung socket does not hang an agent.
const TIMEOUT: Duration = Duration::from_secs(300);

pub fn endpoint(p: Provider) -> &'static str {
    match p {
        Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta/interactions",
        Provider::Deepgram => "https://api.deepgram.com/v1/listen",
        Provider::Whisper => "",
    }
}

/// Which provider to use: the flag, else the first one that has a key, else a
/// refusal that names every way to fix it.
pub fn choose(flag: Option<Provider>, cfg: &Config, env: impl Fn(&str) -> Option<String>) -> Result<(Provider, Resolved)> {
    if let Some(p) = flag {
        let r = Resolved::get(p, cfg, None, &env);
        if r.key.is_none() && p != Provider::Whisper {
            bail!(
                "asr_no_key: no key for {}; run `splice config set {}.api_key --stdin` or set {}",
                p.id(),
                p.id(),
                p.key_env().unwrap_or("the provider's variable")
            );
        }
        return Ok((p, r));
    }
    for p in [Provider::Gemini, Provider::Deepgram] {
        let r = Resolved::get(p, cfg, None, &env);
        if r.key.is_some() {
            return Ok((p, r));
        }
    }
    bail!(
        "asr_no_key: no provider is configured; run `splice config set gemini.api_key --stdin` or `splice config set deepgram.api_key --stdin`"
    )
}

/// Deepgram takes the audio as the body, with everything else in the query.
/// `diarize` is deliberately absent: `diarize=true` is deprecated and routes to
/// the v1 diariser, which collapsed both speakers into one on the sample this was
/// built against. `diarize_model=latest` is the working form, and it is not on by
/// default.
pub fn deepgram_url(model: &str, language: Option<&str>) -> String {
    let mut q = format!(
        "{}?model={}&punctuate=true&smart_format=true&utterances=true&detect_language=true",
        endpoint(Provider::Deepgram),
        urlencode(model)
    );
    if let Some(l) = language {
        q.push_str(&format!("&language={}", urlencode(l)));
    }
    q
}

/// Gemini wants inline base64 under 20 MB and a transcription_config that asks
/// for word timestamps in verbatim mode. `smart` mode cannot carry timestamps, so
/// cleanup happens in `cues()` instead.
pub fn gemini_body(model: &str, audio: &[u8]) -> Value {
    let b64 = base64::engine::general_purpose::STANDARD.encode(audio);
    json!({
        "model": format!("models/{model}"),
        "input": [{"type": "audio", "data": b64, "mime_type": "audio/mp3"}],
        "generation_config": {
            "transcription_config": {
                "mode": {"type": "verbatim", "timestamp_granularities": ["word"]}
            }
        }
    })
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32 as u8),
        })
        .collect()
}

pub fn transcribe(p: Provider, model: &str, language: Option<&str>, audio: &[u8], key: &Secret) -> Result<Parsed> {
    let (status, body) = match p {
        Provider::Gemini => {
            let body = gemini_body(model, audio);
            post_json(endpoint(p), "x-goog-api-key", key.expose(), &body)?
        }
        Provider::Deepgram => post_bytes(
            &deepgram_url(model, language),
            "audio/mp3",
            key.expose(),
            audio,
        )?,
        Provider::Whisper => bail!(
            "asr_unsupported: whisper runs as an external binary and is not wired up yet; use `splice index --from <whisper.json>`"
        ),
    };
    if !(200..300).contains(&status) {
        return Err(error_for(p, status, &body));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("asr_unreadable: {} returned non-JSON: {e}", p.id()))?;
    crate::transcript::parse_value(&value).map_err(|e| {
        anyhow::anyhow!("asr_unreadable: {} returned something splice cannot read: {e}", p.id())
    })
}

/// Turns a provider failure into a code a caller can act on. The bodies differ:
/// Gemini puts the reason in `error.details[].reason` and answers a bad key with
/// **400**, not 401; Deepgram answers `{err_code, err_msg}`.
pub fn error_for(p: Provider, status: u16, body: &str) -> anyhow::Error {
    let detail = match p {
        Provider::Gemini => {
            // The Interactions endpoint answers errors with a JSON *array*
            // containing one object, not the object itself — a fact only a real
            // failure reveals.
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let obj = match &v {
                Value::Array(items) => items.first().cloned().unwrap_or(Value::Null),
                other => other.clone(),
            };
            let reason = obj
                .pointer("/error/details/0/reason")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let msg = obj
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("");
            format!("{reason} {msg}").trim().to_string()
        }
        _ => {
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let code = v.get("err_code").and_then(|c| c.as_str()).unwrap_or("");
            let msg = v.get("err_msg").and_then(|m| m.as_str()).unwrap_or("");
            format!("{code} {msg}").trim().to_string()
        }
    };
    // Match on the body before the status: Gemini answers a bad key with 400,
    // and its own reason string is the only reliable signal.
    let looks_like_key = detail.contains("API_KEY_INVALID")
        || detail.contains("API key not valid")
        || detail.contains("INVALID_AUTH")
        || detail.contains("invalid api key");
    let code = match status {
        _ if looks_like_key => "asr_key_invalid",
        401 | 402 | 403 => "asr_key_invalid",
        400 => "asr_bad_request",
        429 => "asr_rate_limited",
        413 => "asr_too_large",
        s if s >= 500 => "asr_unavailable",
        _ => "asr_failed",
    };
    let fix = match code {
        "asr_key_invalid" => Some("check the key: `splice config list`"),
        "asr_rate_limited" => Some("wait and retry, or use another provider"),
        "asr_too_large" => Some("transcribe a shorter range"),
        _ => None,
    };
    // One layer, code first: report::fail_from classifies on the prefix, and a
    // context layer would put the fix in front of the code it explains.
    match fix {
        Some(f) => anyhow::anyhow!("{code}: {} returned {status}: {detail} (fix: {f})", p.id()),
        None => anyhow::anyhow!("{code}: {} returned {status}: {detail}", p.id()),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

fn post_json(url: &str, header: &str, value: &str, body: &Value) -> Result<(u16, String)> {
    let res = agent()
        .post(url)
        .header(header, value)
        .header("content-type", "application/json")
        .send(body.to_string())
        .with_context(|| format!("could not reach {url}"))?;
    read(res)
}

fn post_bytes(url: &str, content_type: &str, token: &str, audio: &[u8]) -> Result<(u16, String)> {
    let res = agent()
        .post(url)
        .header("authorization", &format!("Token {token}"))
        .header("content-type", content_type)
        .send(audio)
        .with_context(|| format!("could not reach {url}"))?;
    read(res)
}

fn read(mut res: ureq::http::Response<ureq::Body>) -> Result<(u16, String)> {
    let status = res.status().as_u16();
    let body = res
        .body_mut()
        .read_to_string()
        .context("reading the response body")?;
    Ok((status, body))
}

/// The 10-minute confirmation cap, from the design: a long paid upload should be
/// a decision, not a surprise.
pub fn check_duration(duration: Time, confirmed: bool, max: Time) -> Result<()> {
    if duration > max && !confirmed {
        bail!(
            "asr_over_limit: audio is {duration}, limit is {max}; pass --yes to confirm, or --max-duration to raise the limit"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(p: Provider, key: &str) -> Config {
        let mut c = Config::default();
        c.provider_mut(p).api_key = Some(Secret::new(key));
        c
    }

    #[test]
    fn the_flag_wins_and_a_missing_key_is_named() {
        let cfg = cfg_with(Provider::Deepgram, "k");
        let (p, _) = choose(Some(Provider::Deepgram), &cfg, |_| None).unwrap();
        assert_eq!(p, Provider::Deepgram);

        let err = choose(Some(Provider::Gemini), &cfg, |_| None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("asr_no_key"), "{err}");
        assert!(err.contains("GEMINI_API_KEY"), "{err}");
    }

    #[test]
    fn without_a_flag_the_first_configured_provider_wins() {
        let only_deepgram = cfg_with(Provider::Deepgram, "k");
        assert_eq!(choose(None, &only_deepgram, |_| None).unwrap().0, Provider::Deepgram);

        let both = {
            let mut c = cfg_with(Provider::Deepgram, "k");
            c.provider_mut(Provider::Gemini).api_key = Some(Secret::new("g"));
            c
        };
        assert_eq!(choose(None, &both, |_| None).unwrap().0, Provider::Gemini);
    }

    #[test]
    fn the_environment_counts_as_configured() {
        let empty = Config::default();
        let (p, r) = choose(None, &empty, |v| {
            (v == "DEEPGRAM_API_KEY").then(|| "env-key".into())
        })
        .unwrap();
        assert_eq!(p, Provider::Deepgram);
        assert_eq!(r.key.unwrap().expose(), "env-key");
    }

    #[test]
    fn nothing_configured_says_exactly_what_to_run() {
        let err = choose(None, &Config::default(), |_| None).unwrap_err().to_string();
        assert!(err.contains("config set gemini.api_key"), "{err}");
        assert!(err.contains("config set deepgram.api_key"), "{err}");
    }

    #[test]
    fn the_deepgram_url_carries_the_model_and_no_deprecated_diarize() {
        let u = deepgram_url("nova-3", Some("en-GB"));
        assert!(u.starts_with("https://api.deepgram.com/v1/listen?model=nova-3"));
        assert!(u.contains("&language=en-GB"));
        assert!(!u.contains("diarize=true"), "diarize=true is the v1 trap");
    }

    #[test]
    fn the_gemini_body_asks_for_verbatim_words() {
        let b = gemini_body("gemini-3.5-transcribe", b"audio-bytes");
        assert_eq!(b["model"], "models/gemini-3.5-transcribe");
        assert_eq!(
            b.pointer("/generation_config/transcription_config/mode/type")
                .unwrap(),
            "verbatim"
        );
        assert_eq!(
            b.pointer("/generation_config/transcription_config/mode/timestamp_granularities/0")
                .unwrap(),
            "word"
        );
        assert_eq!(b.pointer("/input/0/mime_type").unwrap(), "audio/mp3");
        assert!(b.pointer("/input/0/data").unwrap().as_str().unwrap().len() > 8);
    }

    #[test]
    fn a_gemini_bad_key_is_a_400_with_the_reason_in_details() {
        let body = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"API_KEY_INVALID"}]}}"#;
        let e = error_for(Provider::Gemini, 400, body).to_string();
        assert!(e.starts_with("asr_key_invalid"), "{e}");
    }

    #[test]
    fn deepgram_errors_use_their_own_fields() {
        let e = error_for(
            Provider::Deepgram,
            400,
            r#"{"err_code":"INVALID_QUERY_PARAMETER","err_msg":"Keywords are not supported for Nova-3"}"#,
        )
        .to_string();
        assert!(e.contains("asr_bad_request"), "{e}");
        assert!(e.contains("Nova-3"), "{e}");

        let rate = error_for(Provider::Deepgram, 429, "{}").to_string();
        assert!(rate.starts_with("asr_rate_limited"), "{rate}");
    }

    #[test]
    fn the_ten_minute_cap_needs_confirmation() {
        let ten = Time::from_secs_f64(600.0);
        let long = Time::from_secs_f64(601.0);
        assert!(check_duration(ten, false, ten).is_ok());
        let e = check_duration(long, false, ten).unwrap_err().to_string();
        assert!(e.contains("asr_over_limit"), "{e}");
        assert!(e.contains("--yes"), "{e}");
        assert!(check_duration(long, true, ten).is_ok());
    }
}
