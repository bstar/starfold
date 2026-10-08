use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: Option<u64>,
    pub directory: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarZst,
    TarXz,
    TarBz2,
    SevenZip,
    Rar,
    Native,
}
impl Format {
    pub fn from_path(path: &Path) -> Option<Self> {
        let n = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        [
            (".tar.gz", Self::TarGz),
            (".tgz", Self::TarGz),
            (".tar.zst", Self::TarZst),
            (".tzst", Self::TarZst),
            (".tar.xz", Self::TarXz),
            (".txz", Self::TarXz),
            (".tar.bz2", Self::TarBz2),
            (".tbz2", Self::TarBz2),
            (".tar", Self::Tar),
            (".cbz", Self::Zip),
            (".cbr", Self::Rar),
            (".zip", Self::Zip),
            (".7z", Self::SevenZip),
            (".rar", Self::Rar),
            (".zipx", Self::Native),
            (".ar", Self::Native),
            (".deb", Self::Native),
            (".arj", Self::Native),
            (".cab", Self::Native),
            (".chm", Self::Native),
            (".cpio", Self::Native),
            (".dmg", Self::Native),
            (".exe", Self::Native),
            (".fat", Self::Native),
            (".gz", Self::Native),
            (".bz2", Self::Native),
            (".iso", Self::Native),
            (".lha", Self::Native),
            (".lzh", Self::Native),
            (".lz4", Self::Native),
            (".lzx", Self::Native),
            (".msi", Self::Native),
            (".ntfs", Self::Native),
            (".pkg", Self::Native),
            (".qcow2", Self::Native),
            (".rpm", Self::Native),
            (".sea", Self::Native),
            (".sit", Self::Native),
            (".sitx", Self::Native),
            (".squashfs", Self::Native),
            (".vdi", Self::Native),
            (".vhd", Self::Native),
            (".vhdx", Self::Native),
            (".vmdk", Self::Native),
            (".wim", Self::Native),
            (".xar", Self::Native),
            (".xz", Self::Native),
            (".z", Self::Native),
            (".zst", Self::Native),
            (".001", Self::Native),
        ]
        .into_iter()
        .find(|(ext, _)| n.ends_with(ext))
        .map(|(_, f)| f)
    }
    pub fn writable(self) -> bool {
        !matches!(self, Self::Rar | Self::TarXz | Self::TarBz2 | Self::Native)
    }
}

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    Dir,
    File(u64),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub from: PathBuf,
    pub to: Option<PathBuf>,
    pub kind: ItemKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    Store,
    Fast,
    #[default]
    Balanced,
    Maximum,
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub preset: Preset,
    pub password: Option<String>,
    pub volume_bytes: Option<u64>,
    pub exclude_mac_metadata: bool,
}
impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("preset", &self.preset)
            .field("encrypted", &self.password.is_some())
            .field("volume_bytes", &self.volume_bytes)
            .field("exclude_mac_metadata", &self.exclude_mac_metadata)
            .finish()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub index: usize,
    pub name: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
pub enum Request {
    List {
        source: PathBuf,
        limit: usize,
        password: Option<String>,
    },
    Read {
        source: PathBuf,
        index: usize,
        password: Option<String>,
    },
    Create {
        format: Format,
        output: PathBuf,
        items: Vec<Item>,
        options: Options,
    },
    Extract {
        source: PathBuf,
        output: PathBuf,
        password: Option<String>,
    },
    Rebuild {
        source: PathBuf,
        output: PathBuf,
        changes: Vec<Change>,
        additions: Vec<Item>,
    },
    Test {
        source: PathBuf,
        password: Option<String>,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    Hello {
        version: u32,
        capabilities: Vec<String>,
    },
    Entries {
        entries: Vec<Entry>,
        partial: bool,
    },
    Data {
        length: u32,
    },
    Progress(u64),
    Done,
    Error {
        message: String,
    },
}
pub fn send<T: Serialize>(output: &mut impl Write, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(
        bytes.len() <= MAX_FRAME,
        "Archive protocol frame exceeds limit"
    );
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}
pub fn receive<T: serde::de::DeserializeOwned>(input: &mut impl Read) -> anyhow::Result<T> {
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    anyhow::ensure!(length <= MAX_FRAME, "Archive protocol frame exceeds limit");
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
