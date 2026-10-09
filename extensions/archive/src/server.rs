use starfold_archive_protocol::{receive, send, Reply, Request, VERSION};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
pub fn serve_stdio() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 768 * 1024 * 1024,
            rlim_max: 768 * 1024 * 1024,
        };
        libc::setrlimit(libc::RLIMIT_AS, &limit);
    }
    let mut input = std::io::stdin().lock();
    send(
        &mut std::io::stdout().lock(),
        &Reply::Hello {
            version: VERSION,
            capabilities: [
                "inspect",
                "zip_replace",
                "zip_convert",
                "index",
                "member_read",
                "extract",
                "create",
                "zip_edit",
                "test",
                "password",
                "volumes",
            ]
            .map(str::to_string)
            .to_vec(),
        },
    )?;
    let request: Request = receive(&mut input)?;
    drop(input);
    let progress = Arc::new(crate::progress::Progress::default());
    let running = Arc::new(AtomicBool::new(true));
    let p = progress.clone();
    let live = running.clone();
    let monitor = std::thread::spawn(move || {
        let mut last = 0;
        while live.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
            let done = p.done();
            if done != last {
                let _ = send(&mut std::io::stdout().lock(), &Reply::Progress(done));
                last = done;
            }
        }
    });
    let result = (|| -> anyhow::Result<()> {
        match request {
            Request::ConvertToZip {
                source,
                output,
                password,
            } => {
                crate::native::convert_to_zip(&source, &output, password.as_deref(), &progress)?;
            }
            Request::Inspect { source } => {
                let inspection = crate::native::inspect(&source)?;
                let bytes = serde_json::to_vec(&inspection)?;
                let mut out = std::io::stdout().lock();
                send(
                    &mut out,
                    &Reply::Data {
                        length: bytes.len() as u32,
                    },
                )?;
                out.write_all(&bytes)?;
            }
            Request::RebuildEdited {
                source,
                output,
                changes,
                additions,
                replacements,
                password,
            } => {
                crate::native::rebuild_edited(
                    &source,
                    &output,
                    &changes,
                    &additions,
                    &replacements,
                    password.as_deref(),
                    &progress,
                )?;
            }
            Request::List {
                source,
                limit,
                password,
            } => {
                let (entries, partial) = crate::native::list(&source, limit, password.as_deref())?;
                send(
                    &mut std::io::stdout().lock(),
                    &Reply::Entries { entries, partial },
                )?;
            }
            Request::Read {
                source,
                index,
                password,
            } => {
                struct Stream;
                impl Write for Stream {
                    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                        let b = &b[..b.len().min(256 * 1024)];
                        let mut out = std::io::stdout().lock();
                        send(
                            &mut out,
                            &Reply::Data {
                                length: b.len() as u32,
                            },
                        )
                        .map_err(std::io::Error::other)?;
                        out.write_all(b)?;
                        out.flush()?;
                        Ok(b.len())
                    }
                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }
                crate::native::copy_member(&source, index, &mut Stream, password.as_deref())?;
            }
            Request::Create {
                format,
                output,
                items,
                options,
            } => {
                crate::native::create(format, &output, &items, &options, &progress)?;
            }
            Request::Extract {
                source,
                output,
                password,
            } => {
                std::fs::create_dir(&output)?;
                crate::native::extract(&source, &output, password.as_deref(), &progress)?;
            }
            Request::Rebuild {
                source,
                output,
                changes,
                additions,
            } => {
                crate::native::rebuild(&source, &output, &changes, &additions, &progress)?;
            }
            Request::Test { source, password } => {
                crate::native::test(&source, password.as_deref(), &progress)?
            }
        }
        Ok(())
    })();
    running.store(false, Ordering::Relaxed);
    let _ = monitor.join();
    send(
        &mut std::io::stdout().lock(),
        &match result {
            Ok(()) => Reply::Done,
            Err(error) => Reply::Error {
                message: error.to_string(),
            },
        },
    )?;
    Ok(())
}
