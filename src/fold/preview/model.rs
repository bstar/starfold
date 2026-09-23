//! Backend-independent values crossing the preview connection.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page {
    pub number: u32,
    pub text: String,
    pub truncated: bool,
}
pub use crate::fold::archive::Entry as ArchiveEntry;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Content {
    Metadata,
    Pages(Vec<Page>),
    Archive(Vec<ArchiveEntry>),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub kind: String,
    pub fields: Vec<Field>,
    pub content: Content,
    pub total_pages: Option<u32>,
    pub next_page: Option<u32>,
    pub notice: Option<String>,
}
impl Document {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            fields: vec![],
            content: Content::Metadata,
            total_pages: None,
            next_page: None,
            notice: None,
        }
    }
    pub fn field(&mut self, label: impl Into<String>, value: impl ToString) {
        if self.fields.len() >= 128 {
            return;
        }
        let value = clean(&value.to_string(), 4096);
        if !value.is_empty() {
            self.fields.push(Field {
                label: clean(&label.into(), 128),
                value,
            });
        }
    }
    pub fn cost(&self) -> usize {
        self.fields
            .iter()
            .map(|f| f.label.len() + f.value.len())
            .sum::<usize>()
            + match &self.content {
                Content::Metadata => 0,
                Content::Pages(p) => p.iter().map(|p| p.text.len()).sum(),
                Content::Archive(a) => a.iter().map(|a| a.name.len() + 32).sum(),
            }
            + 256
    }
}
/// Strip controls, including terminal escapes and directional overrides.
pub fn clean(s: &str, limit: usize) -> String {
    s.chars()
        .filter(|c| {
            (!c.is_control() || *c == '\n' || *c == '\t')
                && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(limit)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! { #[test] fn metadata_never_contains_terminal_controls(s in ".{0,500}") { let text=clean(&s,40);prop_assert!(text.chars().count()<=40);prop_assert!(text.chars().all(|c|!c.is_control()||c=='\n'||c=='\t')); } }
}
