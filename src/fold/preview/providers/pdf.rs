use super::{Document, PreviewConfig, Session};
use crate::fold::preview::model::{clean, Content, Page};
use std::{
    io::{self, Write},
    path::Path,
};
pub struct Pdf {
    document: lopdf::Document,
    total: u32,
}
impl Pdf {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let document = lopdf::Document::load(path)?;
        anyhow::ensure!(!document.is_encrypted(), "Password-protected PDF");
        let total = document.get_pages().len().try_into()?;
        Ok(Self { document, total })
    }
}
struct Limited {
    bytes: Vec<u8>,
    cap: usize,
    truncated: bool,
}
impl Write for Limited {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = b.len().min(self.cap.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&b[..n]);
        if n < b.len() {
            self.truncated = true;
            return Err(io::Error::other("page text limit"));
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Session for Pdf {
    fn read(&mut self, page: u32, cfg: &PreviewConfig) -> anyhow::Result<Document> {
        let first = page.max(1);
        let end = first.saturating_add(2).min(self.total);
        let mut result = Document::new("PDF");
        result.total_pages = Some(self.total);
        result.field("Pages", self.total);
        let mut pages = vec![];
        for number in first..=end {
            let mut out = Limited {
                bytes: vec![],
                cap: cfg.pdf_page_bytes,
                truncated: false,
            };
            let extraction = pdf_extract::output_doc_page(
                &self.document,
                &mut pdf_extract::PlainTextOutput::new(&mut out as &mut dyn Write),
                number,
            );
            if !out.truncated {
                extraction?;
            }
            let text = clean(&String::from_utf8_lossy(&out.bytes), cfg.pdf_page_bytes);
            pages.push(Page {
                number,
                text,
                truncated: out.truncated,
            });
        }
        result.next_page = (end < self.total).then_some(end + 1);
        result.content = Content::Pages(pages);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_three_pages_then_later_pages_without_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.pdf");
        crate::fold::testing::write_pdf(&path, 8);
        let mut session = Pdf::open(&path).unwrap();
        let first = session.read(1, &PreviewConfig::default()).unwrap();
        assert_eq!(first.total_pages, Some(8));
        assert_eq!(first.next_page, Some(4));
        let Content::Pages(pages) = first.content else {
            panic!()
        };
        assert_eq!(pages.len(), 3);
        assert!(pages[0].text.contains("Page 1 contents"));
        std::fs::remove_file(&path).unwrap();
        let last = session.read(7, &PreviewConfig::default()).unwrap();
        assert_eq!(last.next_page, None);
        let Content::Pages(pages) = last.content else {
            panic!()
        };
        assert_eq!(pages.len(), 2);
        assert!(pages[1].text.contains("Page 8 contents"));
    }
    #[test]
    fn page_text_is_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.pdf");
        crate::fold::testing::write_pdf(&path, 1);
        let mut session = Pdf::open(&path).unwrap();
        let cfg = PreviewConfig {
            pdf_page_bytes: 4,
            ..PreviewConfig::default()
        };
        let d = session.read(1, &cfg).unwrap();
        let Content::Pages(pages) = d.content else {
            panic!()
        };
        assert!(pages[0].truncated);
        assert!(pages[0].text.len() <= 4);
    }
}
