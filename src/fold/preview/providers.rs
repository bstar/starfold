//! Provider registry. Format-specific APIs stop at this boundary.
mod media;
#[cfg(test)]
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
#[cfg(test)]
struct Pdf;
#[cfg(test)]
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
#[cfg(test)]
static PROVIDERS: [&(dyn Provider + Sync); 3] = [&Pdf, &Media, &Archive];
#[cfg(not(test))]
static PROVIDERS: [&(dyn Provider + Sync); 2] = [&Media, &Archive];
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
/// Local movie tags remain useful before the decoder/player starts.
#[cfg(feature = "media")]
pub fn movie_details(path: &Path) -> Document {
    let mut document = media::read(path).unwrap_or_else(|_| metadata(path));
    document.kind = "Movie details".into();
    if !document
        .fields
        .iter()
        .any(|f| f.label.eq_ignore_ascii_case("title"))
    {
        document.field(
            "Title",
            path.file_stem().unwrap_or_default().to_string_lossy(),
        );
    }
    let id = document
        .fields
        .iter()
        .filter(|f| f.label.to_ascii_lowercase().contains("imdb"))
        .find_map(|f| imdb_id(&f.value))
        .or_else(|| imdb_id(&path.file_name().unwrap_or_default().to_string_lossy()));
    if let Some(id) = id {
        document.fields.retain(|f| {
            !(f.label.to_ascii_lowercase().contains("imdb") && imdb_id(&f.value).is_some())
        });
        document.field("IMDb", format!("https://www.imdb.com/title/{id}/"));
    }
    // Keep identifying details visible even in a short preview pane; codec
    // and individual track descriptions follow the movie information.
    document
        .fields
        .sort_by_key(|field| match field.label.to_ascii_lowercase().as_str() {
            "title" => 0,
            "year" | "date_released" => 1,
            "imdb" => 2,
            "rating" | "imdb_rating" => 3,
            "genre" => 4,
            "director" => 5,
            "plot" | "synopsis" | "summary" | "description" => 6,
            "duration" => 7,
            "dimensions" => 8,
            _ => 9,
        });
    document.notice = Some("Enter to play in Preview".into());
    document
}
#[cfg(feature = "media")]
fn imdb_id(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    for (start, _) in value.match_indices("tt") {
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        let digits = bytes[start + 2..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if (7..=10).contains(&digits)
            && bytes
                .get(start + 2 + digits)
                .is_none_or(|c| !c.is_ascii_alphanumeric())
        {
            return Some(value[start..start + 2 + digits].into());
        }
    }
    None
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

#[cfg(all(test, feature = "media"))]
mod movie_tests {
    use super::*;
    #[test]
    fn imdb_links_accept_ids_and_urls_without_guessing_movie_titles() {
        assert_eq!(imdb_id("tt0089839").as_deref(), Some("tt0089839"));
        assert_eq!(
            imdb_id("https://www.imdb.com/title/tt0089839/").as_deref(),
            Some("tt0089839")
        );
        assert_eq!(
            imdb_id("Movie (1985) [imdb-tt0089839].mkv").as_deref(),
            Some("tt0089839")
        );
        assert!(imdb_id("Movie (1985) {tmdb-17824}.mkv").is_none());
        assert!(imdb_id("tt123 tt123456789012 xtt0089839 tt0089839x").is_none());
    }
}
