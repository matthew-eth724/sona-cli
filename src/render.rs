//! Executes a plan, reusing cache entries whose key matches.
use anyhow::{bail, Context, Result};
use std::time::Duration;

use crate::graph::RenderPlan;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Summary {
    pub total: usize,
    pub ran: usize,
    pub reused: usize,
}

fn complete(path: &std::path::Path) -> bool {
    std::fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

pub fn execute(plan: &RenderPlan, timeout: Duration) -> Result<Summary> {
    let mut summary = Summary {
        total: plan.steps.len(),
        ..Default::default()
    };
    for step in &plan.steps {
        if let Some(side) = &step.sidecar {
            crate::cache::write(&side.path, &side.body)
                .with_context(|| format!("writing {}", side.path.display()))?;
        }
        if complete(&step.output) {
            summary.reused += 1;
            continue;
        }
        // ffmpeg creates its output file before it fails. A failed step must not
        // leave a zero-byte artefact that the next run trusts as a cache hit.
        let _ = std::fs::remove_file(&step.output);
        if let Err(e) = crate::ffmpeg::run(&step.invocation, timeout)
            .with_context(|| format!("step `{}` failed", step.label))
        {
            let _ = std::fs::remove_file(&step.output);
            return Err(e);
        }
        if !complete(&step.output) {
            bail!("step `{}` produced no usable output", step.label);
        }
        summary.ran += 1;
    }
    if let Some(parent) = plan.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::copy(plan.last_output(), &plan.out)
        .with_context(|| format!("copying result to {}", plan.out.display()))?;
    Ok(summary)
}
