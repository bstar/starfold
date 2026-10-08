use starfold_archive_protocol::{receive, send, Reply, Request, VERSION};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("starfold-archive {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
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
    let progress = Arc::new(starfold_archive::progress::Progress::default());
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
            Request::List {
                source,
                limit,
                password,
            } => {
                let (entries, partial) =
                    starfold_archive::native::list(&source, limit, password.as_deref())?;
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
                starfold_archive::native::copy_member(
                    &source,
                    index,
                    &mut Stream,
                    password.as_deref(),
                )?;
            }
            Request::Create {
                format,
                output,
                items,
                options,
            } => {
                starfold_archive::native::create(format, &output, &items, &options, &progress)?;
            }
            Request::Extract {
                source,
                output,
                password,
            } => {
                std::fs::create_dir(&output)?;
                starfold_archive::native::extract(
                    &source,
                    &output,
                    password.as_deref(),
                    &progress,
                )?;
            }
            Request::Rebuild {
                source,
                output,
                changes,
                additions,
            } => {
                starfold_archive::native::rebuild(
                    &source, &output, &changes, &additions, &progress,
                )?;
            }
            Request::Test { source, password } => {
                starfold_archive::native::test(&source, password.as_deref(), &progress)?
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
