//! Provider registry. Format-specific APIs stop at this boundary.
mod media;
mod pdf;
use super::{
    model::{Content, Document},
    PreviewConfig,
};
use crate::fold::file_type::{classify, FileType};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub trait Session {
    fn read(&mut self, page: u32, cfg: &PreviewConfig) -> anyhow::Result<Document>;
}
pub trait Provider {
    fn accepts(&self, kind: FileType) -> bool;
    fn open(&self, path: &Path) -> anyhow::Result<Box<dyn Session>>;
}
struct Pdf;
impl Provider for Pdf {
    fn accepts(&self, kind: FileType) -> bool {
        kind == FileType::Pdf
    }
    fn open(&self, path: &Path) -> anyhow::Result<Box<dyn Session>> {
        Ok(Box::new(pdf::Pdf::open(path)?))
    }
}
struct Media;
impl Provider for Media {
    fn accepts(&self, kind: FileType) -> bool {
        matches!(kind, FileType::Audio | FileType::Video)
    }
    fn open(&self, path: &Path) -> anyhow::Result<Box<dyn Session>> {
        Ok(Box::new(Fixed(media::read(path)?)))
    }
}
struct Archive;
impl Provider for Archive {
    fn accepts(&self, kind: FileType) -> bool {
        kind == FileType::Archive
    }
    fn open(&self, path: &Path) -> anyhow::Result<Box<dyn Session>> {
        let (mut entries, partial) = crate::fold::archive::list(path, 400)?;
        for entry in &mut entries {
            entry.name = super::model::clean(&entry.name, 4096);
        }
        let mut doc = Document::new("Archive");
        doc.field("Entries shown", entries.len());
        if partial {
            doc.notice = Some("Partial listing (preview limit)".into());
        }
        doc.content = Content::Archive(entries);
        Ok(Box::new(Fixed(doc)))
    }
}
struct Fixed(Document);
impl Session for Fixed {
    fn read(&mut self, _: u32, _: &PreviewConfig) -> anyhow::Result<Document> {
        Ok(self.0.clone())
    }
}
static PROVIDERS: [&(dyn Provider + Sync); 3] = [&Pdf, &Media, &Archive];
pub fn supported(path: &Path, head: &[u8]) -> bool {
    PROVIDERS.iter().any(|p| p.accepts(classify(path, head)))
}
pub fn open(path: &Path, head: &[u8]) -> anyhow::Result<Box<dyn Session>> {
    let kind = classify(path, head);
    PROVIDERS
        .iter()
        .find(|p| p.accepts(kind))
        .ok_or_else(|| anyhow::anyhow!("No provider for this file"))?
        .open(path)
}
pub fn metadata(path: &Path) -> Document {
    let mut d = Document::new("File metadata");
    d.field("Type", mime_guess::from_path(path).first_or_octet_stream());
    if let Ok(m) = std::fs::symlink_metadata(path) {
        d.field("Size", crate::fold::format::size(m.len()));
        d.field("Permissions", crate::fold::format::mode(m.mode()));
        if let Ok(t) = m.modified() {
            d.field(
                "Modified",
                jiff::Timestamp::try_from(t)
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
            );
        }
    }
    d
}
pub fn build(path: &Path, head: &[u8], page: u32, cfg: &PreviewConfig) -> Option<Document> {
    if !supported(path, head) {
        return None;
    }
    let mut base = metadata(path);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open(path, head).and_then(|mut s| s.read(page, cfg))
    }))
    .unwrap_or_else(|_| Err(anyhow::anyhow!("Preview parser failed")));
    match result {
        Ok(mut d) => {
            d.fields.extend(base.fields);
            Some(d)
        }
        Err(e) => {
            base.notice = Some(super::model::clean(
                &format!("Preview unavailable: {e}"),
                512,
            ));
            Some(base)
        }
    }
}
