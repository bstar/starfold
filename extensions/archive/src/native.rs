//! Bundled native codec adapters. Passwords travel over stdin, never argv.
use crate::{progress::Progress, Entry, Format};
use starfold_archive_protocol::{Change, Item, ItemKind, Options, Preset};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
fn tool(name: &str) -> PathBuf {
    let env = match name {
        "7zz" => "STARFOLD_ARCHIVE_7ZZ",
        "lsar" => "STARFOLD_ARCHIVE_LSAR",
        _ => "STARFOLD_ARCHIVE_UNAR",
    };
    if let Some(path) = std::env::var_os(env) {
        return path.into();
    }
    let adjacent = std::env::current_exe()
        .unwrap_or_default()
        .with_file_name(name);
    if adjacent.is_file() {
        adjacent
    } else {
        let compiled = match name {
            "7zz" => option_env!("STARFOLD_ARCHIVE_7ZZ"),
            "lsar" => option_env!("STARFOLD_ARCHIVE_LSAR"),
            _ => option_env!("STARFOLD_ARCHIVE_UNAR"),
        };
        compiled
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .unwrap_or_else(|| {
                if name == "7zz" {
                    if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
                        std::env::split_paths(&paths)
                            .flat_map(|p| [p.join("7zz"), p.join("7z")])
                            .find(|p| p.is_file())
                    }) {
                        return path;
                    }
                }
                PathBuf::from(name)
            })
    }
}
fn run(mut command: Command, password: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(password) = password {
        let mut input = child.stdin.take().unwrap();
        input.write_all(password.as_bytes())?;
        input.write_all(b"\n")?;
        input.write_all(password.as_bytes())?;
        input.write_all(b"\n")?;
    } else {
        drop(child.stdin.take());
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let result = std::thread::scope(|scope| {
        let errors = scope.spawn(move || {
            let mut bytes = vec![];
            let mut stderr = stderr;
            let mut buffer = [0; 8192];
            loop {
                let n = stderr.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                if bytes.len() < 1024 * 1024 {
                    let available = (1024 * 1024 - bytes.len()).min(n);
                    bytes.extend_from_slice(&buffer[..available]);
                }
            }
            Ok::<_, std::io::Error>(bytes)
        });
        let mut bytes = vec![];
        stdout.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 32 * 1024 * 1024 {
            let _ = child.kill();
        }
        let status = child.wait()?;
        let error = errors.join().unwrap()?;
        anyhow::ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "Archive index exceeds memory budget"
        );
        anyhow::ensure!(
            status.success(),
            "{}",
            String::from_utf8_lossy(&error).trim()
        );
        Ok(bytes)
    });
    result
}
fn seven_list(path: &Path, password: Option<&str>) -> anyhow::Result<Vec<Entry>> {
    let mut command = Command::new(tool("7zz"));
    command
        .args(["l", "-slt", "-ba", "-sccUTF-8", "--"])
        .arg(path);
    let output = String::from_utf8(run(command, password)?)?;
    let mut entries = vec![];
    for block in output.split("\n\n") {
        let fields: std::collections::BTreeMap<_, _> =
            block.lines().filter_map(|l| l.split_once(" = ")).collect();
        let Some(name) = fields.get("Path") else {
            continue;
        };
        if !fields.contains_key("Size") {
            continue;
        }
        anyhow::ensure!(
            !fields.contains_key("Symbolic Link") && !fields.contains_key("Hard Link"),
            "Archive links are unsupported"
        );
        crate::safe_path(name)?;
        let attributes = fields.get("Attributes").copied().unwrap_or("");
        let directory = fields.get("Folder") == Some(&"+") || attributes.starts_with('D');
        entries.push(Entry {
            name: (*name).into(),
            bytes: fields.get("Size").and_then(|s| s.parse().ok()),
            directory,
        });
    }
    anyhow::ensure!(
        !entries.is_empty() || output.trim().is_empty(),
        "Native engine did not provide an archive index"
    );
    Ok(entries)
}
fn xad_list(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let mut command = Command::new(tool("lsar"));
    command.args(["-json", "--"]).arg(path);
    let output = run(command, None)?;
    let value: serde_json::Value = serde_json::from_slice(&output)?;
    let contents = value
        .get("lsarContents")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Invalid XAD archive index"))?;
    let flag = |entry: &serde_json::Value, key: &str| {
        entry
            .get(key)
            .and_then(|v| v.as_bool().or_else(|| v.as_u64().map(|n| n != 0)))
            .unwrap_or(false)
    };
    contents
        .iter()
        .map(|entry| {
            // Expose AppleDouble headers under their original archive names,
            // rather than aliasing a resource fork onto its data-fork ordinal.
            let entry = entry.get("MacOriginalDictionary").unwrap_or(entry);
            let name = entry
                .get("XADFileName")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Archive entry has no name"))?;
            crate::safe_path(name)?;
            anyhow::ensure!(!flag(entry, "XADIsLink"), "Archive links are unsupported");
            Ok(Entry {
                name: name.into(),
                bytes: entry.get("XADFileSize").and_then(|v| v.as_u64()),
                directory: flag(entry, "XADIsDirectory"),
            })
        })
        .collect()
}
pub fn list(
    path: &Path,
    limit: usize,
    password: Option<&str>,
) -> anyhow::Result<(Vec<Entry>, bool)> {
    if crate::detect(path).ok() != Some(Format::Native) {
        if let Ok(result) = crate::list(path, limit) {
            return Ok(result);
        }
    }
    if crate::detect(path).ok() == Some(Format::TarLz4) {
        return Ok((
            vec![Entry {
                name: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                bytes: None,
                directory: false,
            }],
            false,
        ));
    }
    let mut entries = seven_list(path, password).or_else(|seven| {
        xad_list(path).map_err(|xad| anyhow::anyhow!("Cannot index archive: {seven}; {xad}"))
    })?;
    let partial = entries.len() > limit;
    entries.truncate(limit);
    Ok((entries, partial))
}
pub fn copy_member(
    path: &Path,
    index: usize,
    out: &mut dyn Write,
    password: Option<&str>,
) -> anyhow::Result<()> {
    if crate::detect(path).ok() != Some(Format::Native) {
        // A failed decoder may have written bytes: do not concatenate a
        // second engine's output. Choose native before streaming if needed.
        if crate::list(path, crate::MAX_ENTRIES).is_ok() {
            return crate::copy_member(path, index, out, password);
        }
    }
    if crate::detect(path).ok() == Some(Format::TarLz4)
        && crate::list(path, crate::MAX_ENTRIES).is_err()
    {
        anyhow::ensure!(index == 0, "Archive member disappeared");
        let mut input = lz4_flex::frame::FrameDecoder::new(std::fs::File::open(path)?);
        let mut remaining = crate::MAX_OUTPUT;
        let mut buffer = [0; 64 * 1024];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            anyhow::ensure!(n as u64 <= remaining, "Expansion limit exceeded");
            out.write_all(&buffer[..n])?;
            remaining -= n as u64;
        }
        return Ok(());
    }
    let mut fallback_name = None;
    if let Ok(entries) = seven_list(path, password) {
        let selected = entries
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?;
        anyhow::ensure!(!selected.directory, "Select a regular file");
        anyhow::ensure!(
            entries.iter().filter(|e| e.name == selected.name).count() == 1,
            "Duplicate native archive member names require indexed decoding"
        );
        fallback_name = Some(selected.name.clone());
        let mut command = Command::new(tool("7zz"));
        command
            .args(["x", "-so", "-spd", "--"])
            .arg(path)
            .arg(&selected.name);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(password) = password {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("{password}\n").as_bytes())?;
        } else {
            drop(child.stdin.take());
        }
        let mut output = child.stdout.take().unwrap();
        let mut errors = child.stderr.take().unwrap();
        let mut staged = tempfile::tempfile()?;
        let result = std::thread::scope(|scope| -> anyhow::Result<bool> {
            let stderr = scope.spawn(move || {
                let mut bytes = vec![];
                let mut chunk = [0; 8192];
                loop {
                    let n = errors.read(&mut chunk)?;
                    if n == 0 {
                        break;
                    }
                    if bytes.len() < 1024 * 1024 {
                        let available = (1024 * 1024 - bytes.len()).min(n);
                        bytes.extend_from_slice(&chunk[..available]);
                    }
                }
                Ok::<_, std::io::Error>(bytes)
            });
            let copied = std::io::copy(
                &mut output.by_ref().take(crate::MAX_OUTPUT + 1),
                &mut staged,
            );
            if copied
                .as_ref()
                .map_or(true, |size| *size > crate::MAX_OUTPUT)
            {
                let _ = child.kill();
            }
            let status = child.wait();
            let errors = stderr
                .join()
                .map_err(|_| anyhow::anyhow!("Native codec stderr reader failed"))??;
            let size = copied?;
            let status = status?;
            anyhow::ensure!(
                size <= crate::MAX_OUTPUT,
                "Archive expansion limit exceeded"
            );
            if !status.success() && password.is_some() {
                anyhow::bail!("{}", String::from_utf8_lossy(&errors));
            }
            Ok(status.success())
        })?;
        if result {
            use std::io::{Seek, SeekFrom};
            staged.seek(SeekFrom::Start(0))?;
            std::io::copy(&mut staged, out)?;
            return Ok(());
        }
    }
    anyhow::ensure!(
        password.is_none(),
        "Encrypted legacy archives require a compatible password-capable codec"
    );
    let entries = xad_list(path)?;
    let index = if let Some(name) = fallback_name {
        let matching: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == name && !e.directory)
            .collect();
        anyhow::ensure!(
            matching.len() == 1,
            "Native fallback cannot resolve the exact archive member"
        );
        matching[0].0
    } else {
        index
    };
    let selected = entries
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?;
    let folder = tempfile::tempdir()?;
    let mut command = Command::new(tool("unar"));
    command
        .args([
            "-quiet",
            "-no-directory",
            "-forks",
            "hidden",
            "-indexes",
            "-output-directory",
        ])
        .arg(folder.path())
        .arg("--")
        .arg(path)
        .arg(index.to_string());
    run(command, None)?;
    let file = folder.path().join(crate::safe_path(&selected.name)?);
    let meta = std::fs::symlink_metadata(&file)?;
    anyhow::ensure!(meta.is_file(), "Archive member is not a regular file");
    std::io::copy(&mut std::fs::File::open(file)?, out)?;
    Ok(())
}
pub fn extract(
    path: &Path,
    output: &Path,
    password: Option<&str>,
    progress: &Progress,
) -> anyhow::Result<()> {
    if password.is_none()
        && crate::detect(path).ok() != Some(Format::Native)
        && crate::list(path, crate::MAX_ENTRIES).is_ok()
    {
        return crate::extract(path, output, progress);
    }
    let (entries, partial) = list(path, crate::MAX_ENTRIES, password)?;
    anyhow::ensure!(!partial, "Archive entry limit exceeded");
    let mut total = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        if entry.directory && matches!(entry.name.as_str(), "." | "./") {
            continue;
        }
        let target = output.join(crate::safe_path(&entry.name)?);
        if entry.directory {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        total = total
            .checked_add(entry.bytes.unwrap_or(0))
            .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
        anyhow::ensure!(
            total <= crate::MAX_OUTPUT,
            "Archive expansion limit exceeded"
        );
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        struct Count<'a>(&'a mut std::fs::File, &'a Progress, u64);
        impl Write for Count<'_> {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                if b.len() as u64 > self.2 {
                    return Err(std::io::Error::other("Archive expansion limit exceeded"));
                }
                let n = self.0.write(b)?;
                self.1.add(n as u64);
                self.2 -= n as u64;
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.0.flush()
            }
        }
        copy_member(
            path,
            index,
            &mut Count(
                &mut file,
                progress,
                entry.bytes.unwrap_or(crate::MAX_OUTPUT),
            ),
            password,
        )?;
    }
    Ok(())
}
pub fn create(
    format: Format,
    output: &Path,
    items: &[Item],
    options: &Options,
    progress: &Progress,
) -> anyhow::Result<()> {
    let filtered: Vec<_> = items
        .iter()
        .filter(|i| {
            !options.exclude_mac_metadata
                || !i.to.as_ref().is_some_and(|p| {
                    p.components().any(|c| {
                        c.as_os_str() == "__MACOSX"
                            || c.as_os_str() == ".DS_Store"
                            || c.as_os_str().to_string_lossy().starts_with("._")
                    })
                })
        })
        .cloned()
        .collect();
    let items = filtered.as_slice();
    if options.password.is_none() && options.volume_bytes.is_none() {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(output)?;
        return crate::write::create_options(format, file, items, options, progress);
    }
    anyhow::ensure!(
        matches!(format, Format::Zip | Format::SevenZip),
        "Encryption and split volumes are available for ZIP and 7z"
    );
    let stage = tempfile::tempdir_in(output.parent().unwrap())?;
    for item in items {
        let name = item
            .to
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing archive member name"))?;
        let name = crate::safe_path(&name.to_string_lossy())?;
        let to = stage.path().join(name);
        if matches!(item.kind, ItemKind::Dir) {
            std::fs::create_dir_all(to)?;
        } else {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            use std::os::unix::fs::OpenOptionsExt;
            let mut input = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&item.from)?;
            anyhow::ensure!(input.metadata()?.is_file(), "Source is not a regular file");
            let mut output = std::fs::File::create(to)?;
            let mut buffer = [0; 256 * 1024];
            loop {
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                output.write_all(&buffer[..n])?;
                progress.add(n as u64);
            }
        }
    }
    let mut command = Command::new(tool("7zz"));
    command.current_dir(stage.path()).args([
        "a",
        if format == Format::Zip {
            "-tzip"
        } else {
            "-t7z"
        },
    ]);
    if format == Format::SevenZip {
        command.arg("-md=32m");
    }
    command.arg(format!(
        "-mx={}",
        match options.preset {
            Preset::Store => 0,
            Preset::Fast => 1,
            Preset::Balanced => 5,
            Preset::Maximum => 9,
        }
    ));
    if options.password.is_some() {
        command.arg("-p");
        if format == Format::Zip {
            command.arg("-mem=AES256");
        } else {
            command.arg("-mhe=on");
        }
    }
    if let Some(size) = options.volume_bytes {
        anyhow::ensure!(size >= 1024 * 1024, "Split volumes must be at least 1 MiB");
        command.arg(format!("-v{size}b"));
    }
    command.arg("-sas").arg("--").arg(output).arg(".");
    run(command, options.password.as_deref())?;
    for item in items {
        if let ItemKind::File(size) = item.kind {
            progress.add(size);
        }
    }
    Ok(())
}
pub fn inspect(path: &Path) -> anyhow::Result<starfold_archive_protocol::Inspection> {
    let format = crate::detect(path).unwrap_or(Format::Native);
    // Verify native candidates; a renamed executable is not necessarily an archive.
    if format == Format::Native {
        list(path, 1, None)?;
    }
    let mut encrypted = false;
    if format == Format::Zip {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
        for i in 0..zip.len() {
            encrypted |= zip.by_index_raw(i)?.encrypted();
        }
    }
    Ok(starfold_archive_protocol::Inspection {
        format,
        editable: format == Format::Zip,
        encrypted,
    })
}

pub fn rebuild(
    source: &Path,
    output: &Path,
    changes: &[Change],
    additions: &[Item],
    progress: &Progress,
) -> anyhow::Result<()> {
    rebuild_edited(source, output, changes, additions, &[], None, progress)
}

pub fn rebuild_edited(
    source: &Path,
    output: &Path,
    changes: &[Change],
    additions: &[Item],
    replacements: &[starfold_archive_protocol::Replacement],
    password: Option<&str>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let mut original = zip::ZipArchive::new(std::fs::File::open(source)?)?;
    let mut identities = std::collections::HashSet::new();
    for replacement in replacements {
        anyhow::ensure!(
            replacement.index < original.len() && identities.insert(replacement.index),
            "Invalid or duplicate replacement member"
        );
        let meta = std::fs::symlink_metadata(&replacement.from)?;
        anyhow::ensure!(
            meta.is_file() && meta.len() <= crate::MAX_OUTPUT,
            "Invalid replacement file"
        );
    }
    let any_encrypted =
        (0..original.len()).try_fold(false, |encrypted, i| -> anyhow::Result<bool> {
            Ok(encrypted || original.by_index_raw(i)?.encrypted())
        })?;
    let out = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(output)?;
    let mut writer = zip::ZipWriter::new(out);
    writer.set_raw_comment(original.comment().to_vec().into());
    let mut names = std::collections::HashSet::new();
    for index in 0..original.len() {
        let file = original.by_index_raw(index)?;
        let change = changes.iter().find(|c| c.index == index);
        if change.is_some_and(|c| c.name.is_none()) {
            continue;
        }
        let name = change
            .and_then(|c| c.name.as_ref())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.name().into());
        crate::safe_path(&name)?;
        if change.is_some() {
            anyhow::ensure!(
                !names.contains(&name),
                "ZIP edits create duplicate member names"
            );
        }
        names.insert(name.clone());
        if additions
            .iter()
            .any(|i| i.to.as_ref().is_some_and(|p| p.to_string_lossy() == name))
        {
            continue;
        }
        if let Some(replacement) = replacements.iter().find(|r| r.index == index) {
            anyhow::ensure!(
                !file.is_dir(),
                "Cannot replace a directory with editor contents"
            );
            let mut options = file.options();
            if file.encrypted() {
                let password = password.ok_or_else(|| {
                    anyhow::anyhow!("Unlock this ZIP before saving encrypted members")
                })?;
                // Preserve AES strength; upgrading legacy ZipCrypto to AES keeps edits encrypted.
                let mode = aes_strength(file.extra_data()).unwrap_or(zip::AesMode::Aes256);
                options = options.with_aes_encryption(mode, password);
                drop(file);
                original.by_index_decrypt(index, password.as_bytes())?;
            } else {
                drop(file);
            }
            writer.start_file(name, options)?;
            let n = std::io::copy(&mut std::fs::File::open(&replacement.from)?, &mut writer)?;
            progress.add(n);
            continue;
        }
        let size = file.compressed_size();
        writer.raw_copy_file_rename(file, name)?;
        progress.add(size);
    }
    for item in additions {
        let name = item.to.as_ref().unwrap().to_string_lossy();
        crate::safe_path(&name)?;
        let mut options = zip::write::SimpleFileOptions::default()
            .large_file(matches!(item.kind, ItemKind::File(n) if n >= u32::MAX as u64))
            .compression_method(zip::CompressionMethod::Deflated);
        if any_encrypted {
            options = options.with_aes_encryption(
                zip::AesMode::Aes256,
                password.ok_or_else(|| anyhow::anyhow!("Unlock this ZIP before adding members"))?,
            );
        }
        match item.kind {
            ItemKind::Dir => writer.add_directory(name, options)?,
            ItemKind::File(_) => {
                writer.start_file(name, options)?;
                let mut file = std::fs::File::open(&item.from)?;
                let n = std::io::copy(&mut file, &mut writer)?;
                progress.add(n);
            }
        }
    }
    writer.finish()?.sync_all()?;
    Ok(())
}
fn aes_strength(extra: Option<&[u8]>) -> Option<zip::AesMode> {
    let mut extra = extra?;
    while extra.len() >= 4 {
        let id = u16::from_le_bytes([extra[0], extra[1]]);
        let len = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        let data = extra.get(4..4 + len)?;
        if id == 0x9901 && data.len() >= 7 {
            return match data[4] {
                1 => Some(zip::AesMode::Aes128),
                2 => Some(zip::AesMode::Aes192),
                3 => Some(zip::AesMode::Aes256),
                _ => None,
            };
        }
        extra = &extra[4 + len..];
    }
    None
}

pub fn test(path: &Path, password: Option<&str>, progress: &Progress) -> anyhow::Result<()> {
    let (entries, partial) = list(path, crate::MAX_ENTRIES, password)?;
    anyhow::ensure!(!partial, "Archive exceeds test limit");
    struct Sink<'a> {
        left: u64,
        progress: &'a Progress,
    }
    impl Write for Sink<'_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if b.len() as u64 > self.left {
                return Err(std::io::Error::other("Archive expansion limit exceeded"));
            }
            self.left -= b.len() as u64;
            self.progress.add(b.len() as u64);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        left: crate::MAX_OUTPUT,
        progress,
    };
    for (index, entry) in entries.iter().enumerate() {
        if !entry.directory {
            copy_member(path, index, &mut sink, password)?;
        }
    }
    Ok(())
}

/// Conversion is private and bounded; the host owns final publication.
pub fn convert_to_zip(
    source: &Path,
    output: &Path,
    password: Option<&str>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let scratch = tempfile::tempdir_in(output.parent().unwrap_or(Path::new(".")))?;
    let (entries, partial) = list(source, crate::MAX_ENTRIES, password)?;
    anyhow::ensure!(!partial, "Conversion exceeds entry limit");
    let total = entries
        .iter()
        .filter_map(|e| e.bytes)
        .fold(0u64, u64::saturating_add);
    anyhow::ensure!(
        total <= crate::MAX_OUTPUT,
        "Conversion exceeds expansion limit"
    );
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(scratch.path().as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0 {
        let stat = unsafe { stat.assume_init() };
        // statvfs counters differ in width between Linux and macOS.
        #[allow(clippy::unnecessary_cast)]
        let free = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
        anyhow::ensure!(
            total.saturating_mul(2) <= free,
            "Not enough free space to convert this archive"
        );
    }
    let extracted = scratch.path().join("contents");
    std::fs::create_dir(&extracted)?;
    extract(source, &extracted, password, progress)?;
    fn gather(
        root: &Path,
        path: &Path,
        items: &mut Vec<Item>,
        bytes: &mut u64,
        depth: usize,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            depth <= 64 && items.len() < crate::MAX_ENTRIES,
            "Conversion exceeds tree limits"
        );
        let metadata = std::fs::symlink_metadata(path)?;
        let relative = path.strip_prefix(root)?.to_path_buf();
        if metadata.is_dir() {
            if !relative.as_os_str().is_empty() {
                items.push(Item {
                    from: path.into(),
                    to: Some(relative),
                    kind: ItemKind::Dir,
                });
            }
            for child in std::fs::read_dir(path)? {
                gather(root, &child?.path(), items, bytes, depth + 1)?;
            }
        } else {
            anyhow::ensure!(metadata.is_file(), "Conversion does not follow links");
            *bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("Conversion size overflow"))?;
            anyhow::ensure!(*bytes <= crate::MAX_OUTPUT, "Conversion exceeds size limit");
            items.push(Item {
                from: path.into(),
                to: Some(relative),
                kind: ItemKind::File(metadata.len()),
            });
        }
        Ok(())
    }
    let mut items = vec![];
    gather(&extracted, &extracted, &mut items, &mut 0, 0)?;
    create(
        Format::Zip,
        output,
        &items,
        &Options {
            password: password.map(str::to_owned),
            ..Default::default()
        },
        progress,
    )
}
