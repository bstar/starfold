//! Archive operations use the same executable extension as browser requests.
use crate::fold::ops::{progress::Progress, OpKind, Plan};
use std::path::Path;
pub fn run(kind: OpKind, payload: &Path, plan: &Plan, progress: &Progress) -> anyhow::Result<()> {
    match kind {
        OpKind::Compress(format) => super::service::create(
            format,
            payload,
            &plan.items,
            plan.archive_options.clone(),
            progress,
        ),
        OpKind::Extract => super::service::extract(&plan.sources[0], payload, progress),
        _ => anyhow::bail!("Not an archive operation"),
    }
}
