//! Persistent details for a failed operation.

use std::path::PathBuf;

use starkit::chrome::overlay::{self, Anchor};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use crate::fold::ops::{DeleteHow, Op, OpId, OpKind};
use crate::ui::panels::{elide_middle, fit, rgb};
use crate::ui::theme::Theme;

#[derive(Debug)]
pub struct Failure {
    pub lines: Vec<String>,
    pub scroll: usize,
    pub retry: Option<Vec<PathBuf>>,
    pub sudo_retry: Option<OpId>,
}

impl Failure {
    pub fn from_op(op: &Op) -> Self {
        let mut lines = vec![op.title()];
        if op.failed.is_empty() {
            if let Some(reason) = &op.failure {
                lines.push(reason.clone());
            }
        } else {
            lines.push(format!("{} item(s) failed:", op.failed.len()));
            for (path, reason) in &op.failed {
                lines.push(path.display().to_string());
                let detail = reason
                    .strip_prefix(&format!("{}: ", path.display()))
                    .unwrap_or(reason);
                lines.push(format!("  {detail}"));
            }
        }
        let retry_paths: Vec<PathBuf> = op
            .failed
            .iter()
            .filter(|(path, _)| op.sources.contains(path))
            .map(|(path, _)| path.clone())
            .collect();
        let retry = (op.kind == OpKind::Delete(DeleteHow::Permanent)
            && op.label.is_none()
            && !retry_paths.is_empty())
        .then_some(retry_paths);
        let sudo_retry = (retry.is_some()
            && op.failed.iter().any(|(path, reason)| {
                op.sources.contains(path) && crate::fold::elevated::is_permission_error(reason)
            }))
        .then_some(op.id);
        let permission_error = op.failure.as_ref().is_some_and(|reason| {
            let lower = reason.to_ascii_lowercase();
            lower.contains("permission denied") || lower.contains("operation not permitted")
        });
        if permission_error {
            lines.push("Check permissions on the item and its parent folders.".into());
            if retry.is_some() {
                lines.push("After fixing permissions, press r to retry the failed delete.".into());
            } else if op.label.is_some() {
                lines.push("After fixing permissions, use F7 in Places to retry.".into());
            }
        }
        Self {
            lines,
            scroll: 0,
            retry,
            sudo_retry,
        }
    }
}

pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (40, 100), 22, 9, Anchor::Centre)
}

pub fn footer(failure: &Failure) -> &'static str {
    if failure.sudo_retry.is_some() {
        "r retry · s retry with sudo · j/k scroll · esc close"
    } else if failure.retry.is_some() {
        "r retry delete · j/k scroll · esc close"
    } else {
        "j/k scroll · esc close"
    }
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, failure: &Failure) {
    let rr = rect(area);
    let footer = footer(failure);
    let inner = overlay::render(
        rr,
        buf,
        &overlay::Overlay {
            theme,
            title: "operation failed",
            detail: None,
            footer: Some(footer),
        },
    );
    let style = Style::default().fg(rgb(theme.fg));
    for (row, line) in failure
        .lines
        .iter()
        .skip(failure.scroll)
        .take(usize::from(inner.height))
        .enumerate()
    {
        let text = if line.starts_with("  ") {
            fit(line, inner.width)
        } else {
            elide_middle(line, inner.width)
        };
        buf.set_string(inner.x, inner.y + row as u16, text, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::ops::{ConflictPolicy, OpStatus, Queue};

    #[test]
    fn partial_delete_details_keep_every_failed_path_and_offer_retry() {
        let mut queue = Queue::new();
        let id = queue.enqueue(
            OpKind::Delete(DeleteHow::Permanent),
            vec!["/drive/a".into(), "/drive/b".into()],
            None,
            ConflictPolicy::Ask,
        );
        let op = queue.get_mut(id).unwrap();
        op.status = OpStatus::Failed;
        op.failed = vec![
            ("/drive/a".into(), "/drive/a: Permission denied".into()),
            ("/drive/b".into(), "/drive/b: Permission denied".into()),
        ];
        op.failure = Some(op.failed[0].1.clone());
        let details = Failure::from_op(op);
        assert!(details.lines.iter().any(|line| line == "/drive/a"));
        assert!(details.lines.iter().any(|line| line == "/drive/b"));
        assert!(details
            .lines
            .iter()
            .any(|line| line.contains("Check permissions")));
        assert_eq!(
            details.retry,
            Some(vec!["/drive/a".into(), "/drive/b".into()])
        );
        assert_eq!(details.sudo_retry, Some(id));
    }

    #[test]
    fn drive_trash_failure_cannot_retry_by_deleting_the_mount() {
        let mut queue = Queue::new();
        let id = queue.enqueue(
            OpKind::Delete(DeleteHow::Permanent),
            vec!["/drive/.Trash-1000".into()],
            Some("/drive".into()),
            ConflictPolicy::Ask,
        );
        let op = queue.get_mut(id).unwrap();
        op.label = Some("EMPTY TRASH: drive".into());
        op.status = OpStatus::Failed;
        op.failed = vec![("/drive".into(), "Permission denied".into())];
        op.failure = Some("Permission denied".into());
        let details = Failure::from_op(op);
        assert_eq!(details.retry, None);
        assert_eq!(details.sudo_retry, None);
    }

    #[test]
    fn sudo_retry_is_only_for_permission_denied_permanent_deletes() {
        let mut queue = Queue::new();
        let id = queue.enqueue(
            OpKind::Delete(DeleteHow::Permanent),
            vec!["/drive/file".into()],
            None,
            ConflictPolicy::Ask,
        );
        let op = queue.get_mut(id).unwrap();
        op.status = OpStatus::Failed;
        op.failed = vec![("/drive/file".into(), "disk is read-only".into())];
        assert_eq!(Failure::from_op(op).sudo_retry, None);
        op.kind = OpKind::Delete(DeleteHow::Trash);
        op.failed[0].1 = "Permission denied".into();
        assert_eq!(Failure::from_op(op).sudo_retry, None);
    }
}
