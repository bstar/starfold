//! Bounded UTF-8 text previews.
use super::Preview;
/// A head is binary when it either contains a NUL byte -- the one thing a
/// text file never legitimately does -- or is not valid UTF-8 once a
/// trailing sequence that `max_bytes` may have cut mid-character is
/// forgiven. A genuinely invalid byte *earlier* in the head is not forgiven:
/// that is not a cut character, it is not text.
pub(super) fn is_binary(head: &[u8]) -> bool {
    head.contains(&0) || utf8_prefix(head).is_none()
}

/// The longest valid-UTF-8 prefix of `head`, forgiving only an incomplete
/// multi-byte sequence at the very end -- what `max_bytes` cutting a file
/// mid-character looks like. Any other invalid byte returns `None`.
pub(super) fn utf8_prefix(head: &[u8]) -> Option<&str> {
    match std::str::from_utf8(head) {
        Ok(s) => Some(s),
        Err(e) if e.error_len().is_none() => {
            // `error_len` is `None` exactly when the error is "ran out of
            // bytes", i.e. an incomplete sequence at the end.
            std::str::from_utf8(&head[..e.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

pub(super) fn text_preview(
    head: &[u8],
    bytes_capped: bool,
    full_len: u64,
    max_lines: usize,
) -> Preview {
    let text = utf8_prefix(head).unwrap_or_default();
    let mut truncated = bytes_capped || text.len() < head.len();
    let mut out = String::new();
    let mut lines = 0usize;
    for line in text.split_inclusive('\n') {
        if lines >= max_lines {
            truncated = true;
            break;
        }
        out.push_str(line);
        lines += 1;
    }
    Preview::Text {
        head: out,
        truncated,
        bytes: full_len,
        lines,
    }
}
