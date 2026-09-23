use super::{safe_path, Format};
use crate::fold::ops::{progress::Progress, Item, ItemKind};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};
struct Input<'a> {
    file: File,
    progress: &'a Progress,
}
impl Read for Input<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        let n = self.file.read(b)?;
        self.progress.add(n as u64);
        Ok(n)
    }
}
fn name(item: &Item) -> anyhow::Result<String> {
    let s = item
        .to
        .as_ref()
        .and_then(|p| p.to_str())
        .ok_or_else(|| anyhow::anyhow!("Archive names must be UTF-8"))?;
    safe_path(s)?;
    Ok(s.to_string())
}
fn input<'a>(item: &Item, progress: &'a Progress) -> anyhow::Result<Input<'a>> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(Input {
        file: std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&item.from)?,
        progress,
    })
}
fn tar<W: Write>(out: W, items: &[Item], progress: &Progress) -> anyhow::Result<W> {
    let mut a = tar::Builder::new(out);
    for item in items {
        anyhow::ensure!(!progress.is_cancelled(), "cancelled");
        let mut h = tar::Header::new_gnu();
        h.set_mode(if item.kind == ItemKind::Dir {
            0o755
        } else {
            0o644
        });
        h.set_mtime(0);
        match item.kind {
            ItemKind::Dir => {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_size(0);
                h.set_cksum();
                a.append_data(&mut h, name(item)?, std::io::empty())?;
            }
            ItemKind::File(len) => {
                h.set_size(len);
                h.set_entry_type(tar::EntryType::Regular);
                h.set_cksum();
                a.append_data(&mut h, name(item)?, input(item, progress)?)?;
            }
            _ => anyhow::bail!("Archive creation does not follow or store symlinks"),
        }
    }
    Ok(a.into_inner()?)
}
pub fn create(
    format: Format,
    out: File,
    items: &[Item],
    progress: &Progress,
) -> anyhow::Result<()> {
    match format {
        Format::Zip => {
            let mut a = zip::ZipWriter::new(out);
            for item in items {
                anyhow::ensure!(!progress.is_cancelled(), "cancelled");
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                match item.kind {
                    ItemKind::Dir => a.add_directory(name(item)?, opts)?,
                    ItemKind::File(_) => {
                        a.start_file(name(item)?, opts)?;
                        std::io::copy(&mut input(item, progress)?, &mut a)?;
                    }
                    _ => anyhow::bail!("Archive links unsupported"),
                }
            }
            a.finish()?.sync_all()?;
        }
        Format::Tar => {
            tar(out, items, progress)?.sync_all()?;
        }
        Format::TarGz => {
            tar(
                flate2::write::GzEncoder::new(out, flate2::Compression::fast()),
                items,
                progress,
            )?
            .finish()?
            .sync_all()?;
        }
        Format::TarZst => {
            // The Rust encoder consumes a Read. Spool tar privately to disk,
            // keeping memory bounded even for very large source trees.
            let mut spool = tar(tempfile::tempfile()?, items, progress)?;
            spool.seek(SeekFrom::Start(0))?;
            let mut out = out;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ruzstd::encoding::compress(
                    Input {
                        file: spool,
                        progress,
                    },
                    &mut out,
                    ruzstd::encoding::CompressionLevel::Fastest,
                )
            }));
            anyhow::ensure!(result.is_ok(), "Zstandard compression failed");
            out.sync_all()?;
        }
        Format::SevenZip => {
            let mut a = sevenz_rust2::ArchiveWriter::new(out)?;
            for item in items {
                anyhow::ensure!(!progress.is_cancelled(), "cancelled");
                let n = name(item)?;
                match item.kind {
                    ItemKind::Dir => {
                        let mut entry = sevenz_rust2::ArchiveEntry::new_directory(&n);
                        // 0.20.2 write_file_anti_items inverts this bit for
                        // empty streams. Set it here so the emitted archive
                        // contains an ordinary directory, not an anti-item.
                        // Remove with the pinned dependency upgrade.
                        entry.is_anti_item = true;
                        a.push_archive_entry::<File>(entry, None)?;
                    }
                    ItemKind::File(_) => {
                        a.push_archive_entry(
                            sevenz_rust2::ArchiveEntry::new_file(&n),
                            Some(input(item, progress)?),
                        )?;
                    }
                    _ => anyhow::bail!("Archive links unsupported"),
                }
            }
            a.finish()?.sync_all()?;
        }
        _ => anyhow::bail!("This archive format is read-only"),
    }
    Ok(())
}
