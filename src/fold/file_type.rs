//! Cheap, shared classification. Listing icons never open files; preview
//! dispatch refines the same classification with a bounded signature read.
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Audio,
    Video,
    Pdf,
    Archive,
    Image,
    Text,
    Code,
    Font,
    Binary,
}
impl FileType {
    /// Single-cell Unicode symbols, without private-use/Nerd Font requirements.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Audio => "♪",
            Self::Video => "▶",
            Self::Pdf => "▤",
            Self::Archive => "▣",
            Self::Image => "▧",
            Self::Text => "≡",
            Self::Code => "λ",
            Self::Font => "A",
            Self::Binary => "◇",
        }
    }
}
pub fn classify(path: &Path, head: &[u8]) -> FileType {
    use FileType::*;
    if starkit::image::guess_format(head).is_ok() {
        return Image;
    }
    if head.starts_with(b"%PDF-") {
        return Pdf;
    }
    if head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"7z\xbc\xaf\x27\x1c")
        || head.starts_with(b"Rar!\x1a\x07")
        || head.starts_with(&[0x1f, 0x8b])
        || head.get(257..262) == Some(b"ustar")
    {
        return Archive;
    }
    if head.starts_with(b"ID3") || head.starts_with(b"fLaC") {
        return Audio;
    }
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) || head.get(4..8) == Some(b"ftyp") {
        return Video;
    }
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Pdf,
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "tbz2" | "zst" | "tzst" | "7z" | "rar" => {
            Archive
        }
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" | "aiff" | "aif" | "ape" | "wv"
        | "mpc" => Audio,
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "mpeg" | "mpg" | "wmv" => Video,
        "rs" | "c" | "h" | "cpp" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "sh" | "nix"
        | "html" | "css" | "json" | "toml" | "yaml" | "yml" => Code,
        "ttf" | "otf" | "woff" | "woff2" => Font,
        _ => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            match mime.type_().as_str() {
                "image" => Image,
                "audio" => Audio,
                "video" => Video,
                "text" => Text,
                _ => Binary,
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_override_names() {
        assert_eq!(classify(Path::new("x.txt"), b"%PDF-1.7"), FileType::Pdf);
    }
    #[test]
    fn extensions_are_case_insensitive() {
        assert_eq!(classify(Path::new("X.FLAC"), &[]), FileType::Audio);
    }
}
