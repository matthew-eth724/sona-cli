//! One JSON object on stdout, always. Exit codes by class.
use serde_json::{json, Value};
use std::io::Write;

/// Writes one line to stdout and reports whether it landed. A downstream
/// `head` closes the pipe; that is a normal end of output, not a panic.
pub fn emit_line(line: &str) -> bool {
    let out = std::io::stdout();
    let mut lock = out.lock();
    match writeln!(lock, "{line}") {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => false,
        Err(_) => false,
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: String,
    pub at: String,
    pub message: String,
    pub fix: Option<String>,
}

impl Diagnostic {
    pub fn new(code: &str, at: impl Into<String>, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code: code.to_string(),
            at: at.into(),
            message: message.into(),
            fix: None,
        }
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Diagnostic {
        self.fix = Some(fix.into());
        self
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "code": self.code,
            "at": self.at,
            "message": self.message,
        });
        if let Some(fix) = &self.fix {
            v["fix"] = json!(fix);
        }
        v
    }
}

/// Exit 0 with a JSON body. Diagnostics ride along as `warnings`.
pub fn ok(mut data: Value, warnings: &[Diagnostic]) {
    if let Some(obj) = data.as_object_mut() {
        obj.insert("ok".into(), json!(true));
        if !warnings.is_empty() {
            obj.insert(
                "warnings".into(),
                Value::Array(warnings.iter().map(|d| d.to_json()).collect()),
            );
        }
    }
    emit_line(&serde_json::to_string(&data).unwrap_or_else(|_| "{}".into()));
}

/// Exit non-zero with a one-line, actionable error.
pub fn fail(code: &str, message: impl Into<String>, fix: Option<String>) -> ! {
    let mut err = json!({"code": code, "message": message.into()});
    if let Some(fix) = fix {
        err["fix"] = json!(fix);
    }
    emit_line(&serde_json::to_string(&json!({"ok": false, "error": err})).unwrap());
    eprintln!("{code}: {}", err["message"].as_str().unwrap_or(""));
    if let Some(fix) = err.get("fix").and_then(|f| f.as_str()) {
        eprintln!("fix: {fix}");
    }
    std::process::exit(exit_code(code));
}

/// The command that most often clears this failure. Advice, not a promise: it is
/// attached to an error so an agent does not have to guess the next move.
pub fn default_fix(code: &str) -> Option<String> {
    let fix = match code {
        "no_index" => "sona index --from <transcript.json|.srt>",
        "no_history" => "sona history",
        "usage" => "sona <command> --help",
        "media_missing" => "sona list   (shows clips whose source is gone)",
        "ffmpeg_failed" => "sona plan   (shows the argv that failed)",
        "project_invalid" => "sona undo   (restore the last good document)",
        "no_font" => "install a font or fix fontconfig, then sona doctor",
        _ => return None,
    };
    Some(fix.to_string())
}

pub fn exit_code(code: &str) -> i32 {
    match code {
        c if c.starts_with("usage") => 5,
        c if c.starts_with("ffmpeg") => 4,
        c if c.starts_with("media") || c.starts_with("unreadable") => 3,
        _ => 2,
    }
}

/// Turns any error into the right class without inventing a new one.
pub fn fail_from(e: anyhow::Error) -> ! {
    let text = format!("{e:#}");
    let declared = text
        .split_once(':')
        .map(|(head, _)| head)
        .filter(|head| {
            head.len() > 3
                && head
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
        });
    let code = if let Some(declared) = declared {
        // `bail!("no_font: …")` is how a lower layer names its own failure class.
        match declared {
            "usage" => "usage",
            other => other,
        }
    } else if text.starts_with("usage") {
        "usage"
    } else if text.contains("does not exist")
        || text.contains("no such file")
        || text.starts_with("media_missing")
    {
        "media_missing"
    } else if text.contains("ffmpeg") {
        "ffmpeg_failed"
    } else {
        "project_invalid"
    };
    // `bail!("media_missing: …")` should read as one prefix, not two.
    let first = text.lines().next().unwrap_or("failed");
    let message = first
        .split_once(": ")
        .filter(|(head, _)| *head == code || head.starts_with(code))
        .map(|(_, rest)| rest)
        .unwrap_or(first)
        .to_string();
    fail(code, message, default_fix(code))
}
