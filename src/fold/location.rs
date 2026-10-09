//! Typed browser locations. Archive keys are opaque controller identities,
//! never paths accepted by a filesystem backend.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
const PREFIX: &str = "/__starfold_archive__";
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Member {
    pub index: usize,
    pub name: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArchiveSource {
    pub file: PathBuf,
    pub nested: Vec<Member>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    Filesystem(PathBuf),
    Archive {
        source: ArchiveSource,
        directory: PathBuf,
        member: Option<Member>,
    },
}
impl Location {
    pub fn from_key(key: &Path) -> anyhow::Result<Self> {
        if !is_archive(key) {
            return Ok(Self::Filesystem(key.into()));
        }
        let mut components = key.strip_prefix(PREFIX)?.components();
        let token = components
            .next()
            .ok_or_else(|| anyhow::anyhow!("Missing archive identity"))?
            .as_os_str()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid archive identity"))?;
        anyhow::ensure!(
            token.len() <= 32768 && token.len() % 2 == 0,
            "Invalid archive identity"
        );
        let bytes = (0..token.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&token[i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()?;
        let location: Self = serde_json::from_slice(&bytes)?;
        let Self::Archive {
            source,
            mut directory,
            member,
        } = location
        else {
            anyhow::bail!("Invalid archive location");
        };
        anyhow::ensure!(
            source.nested.len() <= 8,
            "Nested archive depth exceeds eight levels"
        );
        // The first trailing component is the display name, not identity.
        components.next();
        for part in components {
            anyhow::ensure!(member.is_none(), "Cannot create a child of an archive file");
            directory.push(part.as_os_str());
        }
        Ok(Self::Archive {
            source,
            directory,
            member,
        })
    }
    pub fn key(&self) -> PathBuf {
        match self {
            Self::Filesystem(path) => path.clone(),
            Self::Archive {
                source,
                directory,
                member,
            } => {
                let bytes = serde_json::to_vec(self).expect("archive location is serializable");
                let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                let name = member
                    .as_ref()
                    .map(|m| m.name.as_path())
                    .filter(|p| !p.as_os_str().is_empty())
                    .or_else(|| (!directory.as_os_str().is_empty()).then_some(directory.as_path()))
                    .or_else(|| source.nested.last().map(|m| m.name.as_path()))
                    .unwrap_or(&source.file);
                Path::new(PREFIX)
                    .join(token)
                    .join(name.file_name().unwrap_or_default())
            }
        }
    }
    pub fn enter_archive(&self) -> anyhow::Result<Self> {
        let source = match self {
            Self::Filesystem(path) => ArchiveSource {
                file: path.clone(),
                nested: vec![],
            },
            Self::Archive {
                source,
                member: Some(member),
                ..
            } => {
                let mut source = source.clone();
                anyhow::ensure!(
                    source.nested.len() < 8,
                    "Nested archive depth exceeds eight levels"
                );
                source.nested.push(member.clone());
                source
            }
            _ => anyhow::bail!("Select an archive file"),
        };
        anyhow::ensure!(
            source.file.to_str().is_some(),
            "Archive paths must be valid UTF-8"
        );
        Ok(Self::Archive {
            source,
            directory: PathBuf::new(),
            member: None,
        })
    }
    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Filesystem(path) => path.parent().map(|p| Self::Filesystem(p.into())),
            Self::Archive {
                source,
                directory,
                member: Some(_),
            } => Some(Self::Archive {
                source: source.clone(),
                directory: directory.clone(),
                member: None,
            }),
            Self::Archive {
                source,
                directory,
                member: None,
            } if !directory.as_os_str().is_empty() => Some(Self::Archive {
                source: source.clone(),
                directory: directory.parent().unwrap_or(Path::new("")).into(),
                member: None,
            }),
            Self::Archive { source, .. } => {
                let mut source = source.clone();
                if let Some(member) = source.nested.pop() {
                    Some(Self::Archive {
                        source,
                        directory: member.name.parent().unwrap_or(Path::new("")).into(),
                        member: None,
                    })
                } else {
                    source.file.parent().map(|p| Self::Filesystem(p.into()))
                }
            }
        }
    }
    pub fn display(&self) -> String {
        match self {
            Self::Filesystem(path) => path.display().to_string(),
            Self::Archive {
                source,
                directory,
                member,
            } => {
                let mut text = source.file.display().to_string();
                for nested in &source.nested {
                    text.push_str(" / ");
                    text.push_str(&nested.name.display().to_string());
                }
                text.push_str(" / ");
                text.push_str(
                    &member
                        .as_ref()
                        .map(|m| &m.name)
                        .unwrap_or(directory)
                        .display()
                        .to_string(),
                );
                text
            }
        }
    }
}
pub fn is_archive(path: &Path) -> bool {
    path.starts_with(PREFIX)
}
pub fn display(path: &Path) -> String {
    Location::from_key(path)
        .map(|l| l.display())
        .unwrap_or_else(|_| "Invalid archive location".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn member_identity_distinguishes_duplicate_names_and_round_trips() {
        let source = ArchiveSource {
            file: "/tmp/a.zip".into(),
            nested: vec![],
        };
        let a = Location::Archive {
            source: source.clone(),
            directory: "folder".into(),
            member: Some(Member {
                index: 2,
                name: "folder/file.txt".into(),
            }),
        };
        let b = Location::Archive {
            source,
            directory: "folder".into(),
            member: Some(Member {
                index: 3,
                name: "folder/file.txt".into(),
            }),
        };
        assert_ne!(a.key(), b.key());
        assert_eq!(Location::from_key(&a.key()).unwrap(), a);
        assert_eq!(a.key().file_name().unwrap(), "file.txt");
        assert_eq!(a.parent().unwrap().display(), "/tmp/a.zip / folder");
    }
}
