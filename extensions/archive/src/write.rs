use super::{safe_path, Format};
use crate::ops::{progress::Progress, Item, ItemKind};
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
        }
    }
    Ok(a.into_inner()?)
}
pub fn create_options(
    format: Format,
    out: File,
    items: &[Item],
    options: &starfold_archive_protocol::Options,
    progress: &Progress,
) -> anyhow::Result<()> {
    match format {
        Format::Zip => {
            let mut a = zip::ZipWriter::new(out);
            for item in items {
                anyhow::ensure!(!progress.is_cancelled(), "cancelled");
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(
                        if options.preset == starfold_archive_protocol::Preset::Store {
                            zip::CompressionMethod::Stored
                        } else {
                            zip::CompressionMethod::Deflated
                        },
                    )
                    .compression_level(
                        if options.preset == starfold_archive_protocol::Preset::Store {
                            None
                        } else {
                            Some(match options.preset {
                                starfold_archive_protocol::Preset::Fast => 1,
                                starfold_archive_protocol::Preset::Maximum => 9,
                                _ => 6,
                            })
                        },
                    );
                match item.kind {
                    ItemKind::Dir => a.add_directory(name(item)?, opts)?,
                    ItemKind::File(_) => {
                        a.start_file(name(item)?, opts)?;
                        std::io::copy(&mut input(item, progress)?, &mut a)?;
                    }
                }
            }
            a.finish()?.sync_all()?;
        }
        Format::Tar => {
            tar(out, items, progress)?.sync_all()?;
        }
        Format::TarGz => {
            tar(
                flate2::write::GzEncoder::new(
                    out,
                    flate2::Compression::new(match options.preset {
                        starfold_archive_protocol::Preset::Store => 0,
                        starfold_archive_protocol::Preset::Fast => 1,
                        starfold_archive_protocol::Preset::Balanced => 6,
                        starfold_archive_protocol::Preset::Maximum => 9,
                    }),
                ),
                items,
                progress,
            )?
            .finish()?
            .sync_all()?;
        }
        Format::TarZst => {
            if options.preset == starfold_archive_protocol::Preset::Store {
                let mut spool = tar(tempfile::tempfile()?, items, progress)?;
                spool.seek(SeekFrom::Start(0))?;
                let mut out = out;
                ruzstd::encoding::compress(
                    spool,
                    &mut out,
                    ruzstd::encoding::CompressionLevel::Uncompressed,
                );
                out.sync_all()?;
            } else {
                let level = match options.preset {
                    starfold_archive_protocol::Preset::Fast => 1,
                    starfold_archive_protocol::Preset::Maximum => 19,
                    _ => 6,
                };
                tar(
                    zstd::stream::write::Encoder::new(out, level)?,
                    items,
                    progress,
                )?
                .finish()?
                .sync_all()?;
            }
        }
        Format::SevenZip => {
            let mut a = sevenz_rust2::ArchiveWriter::new(out)?;
            let method = if options.preset == starfold_archive_protocol::Preset::Store {
                sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::COPY)
            } else {
                sevenz_rust2::encoder_options::Lzma2Options::from_level(match options.preset {
                    starfold_archive_protocol::Preset::Fast => 1,
                    starfold_archive_protocol::Preset::Maximum => 9,
                    _ => 6,
                })
                .into()
            };
            a.set_content_methods(vec![method]);
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
                }
            }
            a.finish()?.sync_all()?;
        }
        _ => anyhow::bail!("This archive format is read-only"),
    }
    Ok(())
}
