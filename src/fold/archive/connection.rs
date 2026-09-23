//! Supervised archive codec worker. Staging ownership remains in the parent,
//! so cancellation or a parser crash cannot publish partial output.
use super::Format;
use crate::fold::ops::{progress::Progress, Item, ItemKind};
use serde::{Deserialize, Serialize};
#[cfg(not(test))]
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[derive(Serialize, Deserialize)]
enum Request {
    Create {
        format: Format,
        output: PathBuf,
        items: Vec<Source>,
    },
    Extract {
        source: PathBuf,
        output: PathBuf,
    },
}
#[derive(Serialize, Deserialize)]
struct Source {
    from: PathBuf,
    name: PathBuf,
    bytes: Option<u64>,
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Progress(u64),
    Done(Result<(), String>),
}

pub fn run(
    kind: crate::fold::ops::OpKind,
    payload: &Path,
    plan: &crate::fold::ops::Plan,
    progress: &Progress,
) -> anyhow::Result<()> {
    use crate::fold::ops::OpKind;
    let request = match kind {
        OpKind::Compress(format) => Request::Create {
            format,
            output: payload.into(),
            items: plan
                .items
                .iter()
                .map(|i| Source {
                    from: i.from.clone(),
                    name: i.to.clone().unwrap_or_default(),
                    bytes: match i.kind {
                        ItemKind::File(n) => Some(n),
                        _ => None,
                    },
                })
                .collect(),
        },
        OpKind::Extract => Request::Extract {
            source: plan.sources[0].clone(),
            output: payload.into(),
        },
        _ => anyhow::bail!("Not an archive operation"),
    };
    #[cfg(test)]
    {
        execute(request, progress)
    }
    #[cfg(not(test))]
    {
        supervise(request, payload, progress)
    }
}
#[cfg(not(test))]
fn supervise(request: Request, payload: &Path, progress: &Progress) -> anyhow::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(payload.parent().unwrap())?;
    serde_json::to_writer(&mut file, &request)?;
    file.flush()?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--archive-worker")
        .arg(file.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let reader = std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        loop {
            let mut line = vec![];
            match output.by_ref().take(8193).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() > 8192 => break,
                Ok(_) => match serde_json::from_slice::<Reply>(&line) {
                    Ok(r) => {
                        if tx.send(r).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
            }
        }
    });
    let result = loop {
        if progress.is_cancelled() {
            break Err(anyhow::anyhow!("cancelled"));
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Reply::Progress(done)) => {
                if done > progress.done() {
                    progress.add(done - progress.done());
                }
            }
            Ok(Reply::Done(result)) => break result.map_err(anyhow::Error::msg),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("Archive worker stopped before completion"))
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}
fn execute(request: Request, progress: &Progress) -> anyhow::Result<()> {
    match request {
        Request::Create {
            format,
            output,
            items,
        } => {
            let items: Vec<_> = items
                .into_iter()
                .map(|s| Item {
                    from: s.from,
                    to: Some(s.name),
                    kind: s.bytes.map(ItemKind::File).unwrap_or(ItemKind::Dir),
                })
                .collect();
            let out = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(output)?;
            super::create(format, out, &items, progress)
        }
        Request::Extract { source, output } => {
            std::fs::create_dir(&output)?;
            super::extract(&source, &output, progress)
        }
    }
}
pub fn child_main(path: &Path) -> anyhow::Result<()> {
    crate::fold::process::limit_memory();
    let request: Request =
        serde_json::from_reader(std::fs::File::open(path)?.take(64 * 1024 * 1024))?;
    let progress = Arc::new(Progress::default());
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let reporter = {
        let p = progress.clone();
        let r = running.clone();
        std::thread::spawn(move || {
            while r.load(std::sync::atomic::Ordering::Relaxed) {
                let mut stdout = std::io::stdout().lock();
                let _ = serde_json::to_writer(&mut stdout, &Reply::Progress(p.done()));
                let _ = stdout.write_all(b"\n");
                let _ = stdout.flush();
                drop(stdout);
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let result = execute(request, &progress).map_err(|e| e.to_string());
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = reporter.join();
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &Reply::Progress(progress.done()))?;
    out.write_all(b"\n")?;
    serde_json::to_writer(&mut out, &Reply::Done(result))?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
