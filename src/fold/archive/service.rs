//! Supervised archive extension client. The default provider ships inside FOLD.
use super::{Entry, Format};
use crate::fold::ops::progress::Progress;
use starfold_archive_protocol::{receive, send, Reply, Request, VERSION};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
pub fn executable() -> PathBuf {
    std::env::var_os("STARFOLD_ARCHIVE_EXTENSION")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let current = std::env::current_exe().unwrap_or_default();
            let folder = current.parent().unwrap_or(Path::new("/"));
            if folder.file_name().is_some_and(|n| n == "deps") {
                folder.parent().unwrap_or(folder).join("starfold-archive")
            } else {
                folder.join("starfold-archive")
            }
        })
}
pub fn request(
    request: Request,
    progress: &Progress,
    mut data: Option<&mut dyn Write>,
) -> anyhow::Result<Option<(Vec<Entry>, bool)>> {
    let required = match &request {
        Request::List { .. } => "index",
        Request::Read { .. } => "member_read",
        Request::Create { .. } => "create",
        Request::Extract { .. } => "extract",
        Request::Rebuild { .. } => "zip_edit",
        Request::Test { .. } => "test",
    };
    let baseline = progress.done();
    let deadline = std::time::Instant::now()
        + if matches!(&request, Request::List { .. }) {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(24 * 60 * 60)
        };
    let mut command = if let Some(explicit) = std::env::var_os("STARFOLD_ARCHIVE_EXTENSION") {
        let mut command = Command::new(explicit);
        command.arg("--stdio");
        command
    } else {
        let current = std::env::current_exe()?;
        // Unit-test harnesses cannot enter main's private provider mode.
        if cfg!(test) {
            let mut command = Command::new(executable());
            command.arg("--stdio");
            command
        } else {
            let mut command = Command::new(current);
            command.arg("--archive-extension-stdio");
            command
        }
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Archive extension unavailable: {e}. Check the configured archive provider or rebuild starfold."
        )
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let pid = child.id();
    let watcher = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !flag.load(Ordering::Relaxed) {
                if progress.is_cancelled() || std::time::Instant::now() >= deadline {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = (|| -> anyhow::Result<Option<(Vec<Entry>, bool)>> {
            let mut output = child.stdout.take().unwrap();
            match receive::<Reply>(&mut output)? {
                Reply::Hello {
                    version,
                    capabilities,
                } if version == VERSION => {
                    anyhow::ensure!(
                        capabilities.iter().any(|c| c == required),
                        "Archive extension does not support {required}"
                    );
                }
                _ => anyhow::bail!("Archive extension protocol mismatch"),
            }
            send(&mut child.stdin.take().unwrap(), &request)?;
            let mut entries = None;
            loop {
                match receive::<Reply>(&mut output)? {
                    Reply::Entries {
                        entries: items,
                        partial,
                    } => entries = Some((items, partial)),
                    Reply::Data { length } => {
                        anyhow::ensure!(
                            length <= 256 * 1024,
                            "Archive extension chunk exceeds limit"
                        );
                        let mut bytes = vec![0; length as usize];
                        output.read_exact(&mut bytes)?;
                        if let Some(out) = data.as_mut() {
                            out.write_all(&bytes)?;
                        } else {
                            anyhow::bail!("Unexpected archive member data");
                        }
                    }
                    Reply::Progress(done) => {
                        let previous = progress.done();
                        if baseline.saturating_add(done) > previous {
                            progress.add(baseline.saturating_add(done) - previous);
                        }
                    }
                    Reply::Done => break,
                    Reply::Error { message } => anyhow::bail!(message),
                    Reply::Hello { .. } => anyhow::bail!("Unexpected archive handshake"),
                }
            }
            Ok(entries)
        })();
        stop.store(true, Ordering::Relaxed);
        result
    });
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.wait();
    if progress.is_cancelled() {
        anyhow::bail!("cancelled");
    }
    if std::time::Instant::now() >= deadline {
        anyhow::bail!("Archive extension timed out");
    }
    watcher
}
pub fn list(path: &Path, limit: usize) -> anyhow::Result<(Vec<Entry>, bool)> {
    list_password(path, limit, None)
}
pub fn list_password(
    path: &Path,
    limit: usize,
    password: Option<&str>,
) -> anyhow::Result<(Vec<Entry>, bool)> {
    request(
        Request::List {
            source: path.into(),
            limit,
            password: password.map(str::to_string),
        },
        &Progress::new(0),
        None,
    )?
    .ok_or_else(|| anyhow::anyhow!("Archive extension returned no index"))
}
pub fn copy_member(
    path: &Path,
    index: usize,
    output: &mut dyn Write,
    password: Option<&str>,
) -> anyhow::Result<()> {
    copy_member_progress(path, index, output, password, &Progress::new(0))
}
pub fn copy_member_progress(
    path: &Path,
    index: usize,
    output: &mut dyn Write,
    password: Option<&str>,
    progress: &Progress,
) -> anyhow::Result<()> {
    request(
        Request::Read {
            source: path.into(),
            index,
            password: password.map(str::to_string),
        },
        progress,
        Some(output),
    )
    .map(|_| ())
}
pub fn extract(path: &Path, output: &Path, progress: &Progress) -> anyhow::Result<()> {
    request(
        Request::Extract {
            source: path.into(),
            output: output.into(),
            password: super::browser::password_for(&crate::fold::location::ArchiveSource {
                file: path.into(),
                nested: vec![],
            }),
        },
        progress,
        None,
    )
    .map(|_| ())
}
pub fn create(
    format: Format,
    output: &Path,
    items: &[crate::fold::ops::Item],
    options: starfold_archive_protocol::Options,
    progress: &Progress,
) -> anyhow::Result<()> {
    let items = items
        .iter()
        .map(|i| starfold_archive_protocol::Item {
            from: i.from.clone(),
            to: i.to.clone(),
            kind: match i.kind {
                crate::fold::ops::ItemKind::Dir => starfold_archive_protocol::ItemKind::Dir,
                crate::fold::ops::ItemKind::File(n) => starfold_archive_protocol::ItemKind::File(n),
                _ => starfold_archive_protocol::ItemKind::File(0),
            },
        })
        .collect();
    request(
        Request::Create {
            format,
            output: output.into(),
            items,
            options,
        },
        progress,
        None,
    )
    .map(|_| ())
}
