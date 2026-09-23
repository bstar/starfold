//! Bounded preview connection. One supervised, disposable parser process owns
//! the current session; the parent owns cancellation, caching and deadlines.
use super::{model::Document, providers, Preview, PreviewConfig};
use serde::{Deserialize, Serialize};
use std::os::unix::{
    ffi::{OsStrExt, OsStringExt},
    fs::MetadataExt,
};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

const MAX_REPLY: u64 = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Request {
    path: Vec<u8>,
    page: u32,
    cfg: PreviewConfig,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    path: PathBuf,
    dev: u64,
    ino: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Identity {
    fn read(path: &Path) -> std::io::Result<Self> {
        let m = std::fs::metadata(path)?;
        Ok(Self {
            path: path.into(),
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
}
struct Parser {
    child: Child,
    input: ChildStdin,
    replies: crossbeam_channel::Receiver<Result<Document, String>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Parser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.reader.take() {
            let _ = t.join();
        }
    }
}
impl Parser {
    fn spawn() -> anyhow::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("--preview-worker");
        Self::start(command)
    }
    fn start(mut command: Command) -> anyhow::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, replies) = crossbeam_channel::bounded(1);
        let reader = std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                match output
                    .by_ref()
                    .take(MAX_REPLY + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() as u64 > MAX_REPLY => {
                        let _ = tx.try_send(Err("Parser response exceeds limit".into()));
                        break;
                    }
                    Ok(_) => {
                        let result = serde_json::from_slice::<Result<Document, String>>(&line)
                            .unwrap_or_else(|e| Err(format!("Invalid parser response: {e}")));
                        if tx.try_send(result).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
            reader: Some(reader),
        })
    }
    fn request(
        &mut self,
        path: &Path,
        page: u32,
        cfg: &PreviewConfig,
        stale: &dyn Fn() -> bool,
    ) -> anyhow::Result<Document> {
        serde_json::to_writer(
            &mut self.input,
            &Request {
                path: path.as_os_str().as_bytes().to_vec(),
                page,
                cfg: *cfg,
            },
        )?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let deadline = Instant::now() + Duration::from_millis(cfg.timeout_ms);
        loop {
            anyhow::ensure!(!stale(), "Preview cancelled");
            anyhow::ensure!(Instant::now() < deadline, "Preview timed out");
            match self.replies.recv_timeout(Duration::from_millis(10)) {
                Ok(r) => return r.map_err(anyhow::Error::msg),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Preview parser stopped")
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
#[derive(Default)]
pub struct Connection {
    parser: Option<(Identity, Parser)>,
    cache: VecDeque<(Identity, u32, Document)>,
}
impl Connection {
    pub fn close(&mut self) {
        self.parser = None;
    }

    pub fn build(
        &mut self,
        path: &Path,
        mut page: u32,
        cfg: &PreviewConfig,
        stale: &dyn Fn() -> bool,
    ) -> Option<Preview> {
        if stale() {
            self.parser = None;
            return None;
        }
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => return Some(Preview::Error(e.to_string())),
        };
        if meta.is_dir() {
            self.close();
            let tree = super::directory::build(path, cfg, stale);
            return (!stale()).then_some(Preview::Dir(tree));
        }
        if !meta.is_file() {
            return Some(super::build(
                path,
                cfg,
                &std::sync::atomic::AtomicBool::new(false),
            ));
        }
        let identity = Identity::read(path).ok()?;
        let changed = self
            .parser
            .as_ref()
            .is_some_and(|(old, _)| old.path == identity.path && *old != identity)
            || self
                .cache
                .iter()
                .any(|(old, _, _)| old.path == identity.path && *old != identity);
        if changed {
            self.cache
                .retain(|(old, _, _)| old.path != identity.path || *old == identity);
            if page > 1 {
                page = 1;
            }
        }

        if let Some(i) = self
            .cache
            .iter()
            .position(|(id, p, _)| *id == identity && *p == page)
        {
            let entry = self.cache.remove(i).unwrap();
            let result = entry.2.clone();
            self.cache.push_back(entry);
            return Some(Preview::Document(result));
        }
        let mut head = vec![0; 512];
        let n = std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        head.truncate(n);
        if !providers::supported(path, &head) {
            self.parser = None;
            return Some(super::build(
                path,
                cfg,
                &std::sync::atomic::AtomicBool::new(false),
            ));
        }
        if !self.parser.as_ref().is_some_and(|(id, _)| *id == identity) {
            self.parser = None;
        }
        let result = (|| {
            if self.parser.is_none() {
                self.parser = Some((identity.clone(), Parser::spawn()?));
            }
            self.parser
                .as_mut()
                .unwrap()
                .1
                .request(path, page, cfg, stale)
        })();
        if stale() {
            self.parser = None;
            return None;
        }
        if Identity::read(path).ok().as_ref() != Some(&identity) {
            self.parser = None;
            return None;
        }
        let d = match result {
            Ok(mut d) => {
                d.fields.extend(providers::metadata(path).fields);
                d
            }
            Err(e) => {
                self.parser = None;
                let mut d = providers::metadata(path);
                d.notice = Some(super::model::clean(&e.to_string(), 512));
                return Some(Preview::Document(d));
            }
        };
        self.cache.push_back((identity, page, d.clone()));
        while self.cache.iter().map(|(_, _, d)| d.cost()).sum::<usize>() > cfg.cache_bytes {
            self.cache.pop_front();
        }
        Some(Preview::Document(d))
    }
}
/// Private protocol, entered before application config, logging or terminal IO.
pub fn child_main() -> anyhow::Result<()> {
    crate::fold::process::limit_memory();
    let mut input = BufReader::new(std::io::stdin());
    let mut output = std::io::stdout().lock();
    let mut session: Option<(Vec<u8>, Box<dyn providers::Session>)> = None;
    loop {
        let mut line = vec![];
        if input.by_ref().take(65537).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        anyhow::ensure!(line.len() <= 65536, "Request too large");
        let request: Request = serde_json::from_slice(&line)?;
        let result = (|| -> anyhow::Result<Document> {
            if !session.as_ref().is_some_and(|(p, _)| *p == request.path) {
                let path = PathBuf::from(std::ffi::OsString::from_vec(request.path.clone()));
                let mut head = vec![0; 512];
                let n = std::fs::File::open(&path)?.read(&mut head)?;
                head.truncate(n);
                session = Some((request.path, providers::open(&path, &head)?));
            }
            session.as_mut().unwrap().1.read(request.page, &request.cfg)
        })()
        .map_err(|e| e.to_string());
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sleeper() -> Parser {
        let mut c = Command::new("sh");
        c.args(["-c", "read request; exec sleep 30"]);
        Parser::start(c).unwrap()
    }
    #[test]
    fn deadline_kills_and_reaps_a_stalled_backend() {
        let mut parser = sleeper();
        let start = Instant::now();
        let cfg = PreviewConfig {
            timeout_ms: 30,
            ..PreviewConfig::default()
        };
        let e = parser
            .request(Path::new("x.pdf"), 1, &cfg, &|| false)
            .unwrap_err();
        assert!(e.to_string().contains("timed out"));
        drop(parser);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn generation_cancellation_interrupts_a_stalled_backend() {
        let mut parser = sleeper();
        let start = Instant::now();
        let e = parser
            .request(Path::new("x.pdf"), 1, &PreviewConfig::default(), &|| {
                start.elapsed() > Duration::from_millis(20)
            })
            .unwrap_err();
        assert!(e.to_string().contains("cancelled"));
        drop(parser);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn cached_result_survives_closed_parser_and_file_changes_invalidate_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("book.pdf");
        std::fs::write(&path, b"%PDF-1.7").unwrap();
        let id = Identity::read(&path).unwrap();
        let doc = Document::new("cached PDF");
        let mut connection = Connection::default();
        connection.cache.push_back((id.clone(), 1, doc));
        let Some(Preview::Document(d)) =
            connection.build(&path, 1, &PreviewConfig::default(), &|| false)
        else {
            panic!()
        };
        assert_eq!(d.kind, "cached PDF");
        assert!(connection.parser.is_none());
        std::fs::write(&path, b"replacement").unwrap();
        assert_ne!(Identity::read(&path).unwrap(), id);
    }
}
