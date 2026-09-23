use super::Entry as ArchiveEntry;
use super::{safe_path, Format, MAX_ENTRIES, MAX_OUTPUT};
use crate::fold::ops::progress::Progress;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

fn tar_reader(path: &Path, format: Format) -> anyhow::Result<Box<dyn Read>> {
    let f = File::open(path)?;
    Ok(match format {
        Format::Tar => Box::new(f),
        Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        Format::TarXz => Box::new(lzma_rust2::XzReader::new(f, true)),
        Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        Format::TarZst => Box::new(ruzstd::decoding::StreamingDecoder::new(f)?),
        _ => unreachable!(),
    })
}
fn entry(name: &str, bytes: u64, directory: bool) -> ArchiveEntry {
    ArchiveEntry {
        name: name.to_string(),
        bytes: Some(bytes),
        directory,
    }
}
pub fn list(path: &Path, limit: usize) -> anyhow::Result<(Vec<ArchiveEntry>, bool)> {
    let format = Format::detect(path)?;
    let limit = limit.min(MAX_ENTRIES);
    let mut entries = vec![];
    match format {
        Format::Zip => {
            let mut a = zip::ZipArchive::new(File::open(path)?)?;
            for i in 0..a.len().min(limit + 1) {
                let e = a.by_index_raw(i)?;
                entries.push(entry(e.name(), e.size(), e.is_dir()));
            }
        }
        Format::SevenZip => {
            let a = sevenz_rust2::Archive::open(path)?;
            for e in a.files.iter().take(limit + 1) {
                entries.push(entry(e.name(), e.size(), e.is_directory()));
            }
        }
        Format::Rar => {
            let mut a = super::rar::Reader::open(path, false)?;
            while entries.len() <= limit {
                let Some(e) = a.next()? else {
                    break;
                };
                entries.push(e);
                a.skip()?;
            }
        }
        _ => {
            let mut a = tar::Archive::new(tar_reader(path, format)?);
            for e in a.entries()?.take(limit + 1) {
                let e = e?;
                entries.push(entry(
                    &e.path()?.to_string_lossy(),
                    e.size(),
                    e.header().entry_type().is_dir(),
                ));
            }
        }
    }
    let partial = entries.len() > limit;
    entries.truncate(limit);
    Ok((entries, partial))
}
/// The destination is an owned, private staging directory. No backend ever
/// receives a final destination or permission to create links/device nodes.
struct Sink<'a> {
    root: &'a Path,
    progress: &'a Progress,
    remaining: u64,
    count: usize,
}
impl Sink<'_> {
    fn begin(&mut self, name: &str, directory: bool, size: u64) -> anyhow::Result<Option<File>> {
        anyhow::ensure!(!self.progress.is_cancelled(), "cancelled");
        self.count += 1;
        anyhow::ensure!(self.count <= MAX_ENTRIES, "Archive entry limit exceeded");
        anyhow::ensure!(size <= self.remaining, "Archive expansion limit exceeded");
        if directory && matches!(name, "." | "./") {
            return Ok(None);
        }
        let to = self.root.join(safe_path(name)?);
        if directory {
            std::fs::create_dir_all(&to)?;
            return Ok(None);
        }
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(Some(
            OpenOptions::new().write(true).create_new(true).open(to)?,
        ))
    }
    fn copy(
        &mut self,
        name: &str,
        directory: bool,
        size: u64,
        input: &mut dyn Read,
    ) -> anyhow::Result<()> {
        let Some(out) = self.begin(name, directory, size)? else {
            return Ok(());
        };
        let mut out = CheckedWriter {
            file: out,
            remaining: &mut self.remaining,
            progress: self.progress,
        };
        std::io::copy(input, &mut out)?;
        Ok(())
    }
}
struct CheckedWriter<'a> {
    file: File,
    remaining: &'a mut u64,
    progress: &'a Progress,
}
impl Write for CheckedWriter<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        if b.len() as u64 > *self.remaining {
            return Err(std::io::Error::other("Archive expansion limit exceeded"));
        }
        let n = self.file.write(b)?;
        *self.remaining -= n as u64;
        self.progress.add(n as u64);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
pub fn extract(path: &Path, root: &Path, progress: &Progress) -> anyhow::Result<()> {
    let format = Format::detect(path)?;
    let mut sink = Sink {
        root,
        progress,
        remaining: MAX_OUTPUT,
        count: 0,
    };
    match format {
        Format::Zip => {
            let mut a = zip::ZipArchive::new(File::open(path)?)?;
            anyhow::ensure!(a.len() <= MAX_ENTRIES, "Archive entry limit exceeded");
            for i in 0..a.len() {
                let mut e = a.by_index(i)?;
                let mode = e.unix_mode().unwrap_or(0) & 0o170000;
                anyhow::ensure!(
                    matches!(mode, 0 | 0o100000 | 0o040000),
                    "Archive links and special files are unsupported"
                );
                anyhow::ensure!(!e.encrypted(), "Encrypted archives are unsupported");
                let (name, dir, size) = (e.name().to_string(), e.is_dir(), e.size());
                sink.copy(&name, dir, size, &mut e)?;
            }
        }
        Format::SevenZip => {
            let mut a = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())?;
            anyhow::ensure!(
                a.archive().files.len() <= MAX_ENTRIES,
                "Archive entry limit exceeded"
            );
            anyhow::ensure!(
                !a.archive()
                    .blocks
                    .iter()
                    .flat_map(|b| &b.coders)
                    .any(|c| c.encoder_method_id() == [6, 0xf1, 7, 1]),
                "Encrypted archives are unsupported"
            );
            a.for_each_entries(|e, input| {
                let mode = (e.windows_attributes() >> 16) & 0o170000;
                if !matches!(mode, 0 | 0o100000 | 0o040000) || e.is_anti_item() {
                    return Err(sevenz_rust2::Error::Other(
                        format!("Archive links and special entries are unsupported: {} mode={mode:o} anti={}", e.name(), e.is_anti_item()).into(),
                    ));
                }
                sink.copy(e.name(), e.is_directory(), e.size(), input)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                Ok(true)
            })?;
        }
        Format::Rar => {
            let mut a = super::rar::Reader::open(path, true)?;
            while let Some(e) = a.next()? {
                if let Some(out) = sink.begin(&e.name, e.directory, e.bytes.unwrap_or(0))? {
                    a.copy(&mut CheckedWriter {
                        file: out,
                        remaining: &mut sink.remaining,
                        progress,
                    })?;
                } else {
                    a.skip()?;
                }
            }
        }
        _ => {
            let mut a = tar::Archive::new(tar_reader(path, format)?);
            for e in a.entries()? {
                let mut e = e?;
                let t = e.header().entry_type();
                anyhow::ensure!(
                    t.is_file() || t.is_dir(),
                    "Archive links and special files are unsupported"
                );
                let name = e
                    .path()?
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Non-UTF-8 archive member name"))?
                    .to_owned();
                let size = e.size();
                sink.copy(&name, t.is_dir(), size, &mut e)?;
            }
        }
    }
    Ok(())
}
