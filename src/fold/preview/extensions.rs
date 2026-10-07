//! Explicit executable providers and their supervised, generation-scoped sessions.
use super::{
    model::{Document, Page},
    Preview, PreviewConfig,
};
use serde::{Deserialize, Serialize};
use starfold_preview_protocol as wire;
use std::{
    collections::BTreeMap,
    io::BufReader,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Registry {
    pub disabled: Vec<String>,
    pub providers: Vec<Provider>,
    /// MIME type -> provider id. Explicit overrides precede ordinary matches.
    pub overrides: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub mime_types: Vec<String>,
}
impl Registry {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for provider in &self.providers {
            anyhow::ensure!(
                !provider.id.is_empty() && provider.id.len() <= 64 && ids.insert(&provider.id),
                "Invalid or duplicate preview provider id"
            );
            anyhow::ensure!(
                provider.command.first().is_some_and(|s| !s.is_empty())
                    && provider.command.len() <= 64,
                "Preview provider needs executable argv"
            );
        }
        for id in self.overrides.values() {
            anyhow::ensure!(
                ["pdf", "video"].contains(&id.as_str())
                    || self.providers.iter().any(|p| &p.id == id),
                "Unknown preview provider override: {id}"
            );
        }
        Ok(())
    }

    pub fn select(&self, path: &Path, head: &[u8]) -> Option<Provider> {
        let kind = crate::fold::file_type::classify(path, head);
        let mime = if kind == crate::fold::file_type::FileType::Pdf {
            "application/pdf".into()
        } else {
            mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string()
        };
        let builtin = match kind {
            crate::fold::file_type::FileType::Pdf => Some("pdf"),
            #[cfg(feature = "media")]
            crate::fold::file_type::FileType::Video => Some("video"),
            _ => None,
        };
        let enabled = |id: &str| !self.disabled.iter().any(|s| s == id);
        let resolve = |id: &str| {
            self.providers
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .or_else(|| {
                    ["pdf", "video"].contains(&id).then(|| Provider {
                        id: id.into(),
                        command: vec![helper_path(id).to_string_lossy().into_owned()],
                        extensions: vec![],
                        mime_types: vec![],
                    })
                })
        };
        if let Some(id) = self.overrides.get(&mime) {
            return enabled(id).then(|| resolve(id)).flatten();
        }
        if let Some(p) = self.providers.iter().find(|p| {
            enabled(&p.id)
                && (p.mime_types.contains(&mime)
                    || path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| {
                            p.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
                        }))
        }) {
            return Some(p.clone());
        }
        builtin.filter(|id| enabled(id)).and_then(resolve)
    }
    pub fn disables_builtin(&self, path: &Path, head: &[u8]) -> bool {
        let mime = if crate::fold::file_type::classify(path, head)
            == crate::fold::file_type::FileType::Pdf
        {
            "application/pdf".into()
        } else {
            mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string()
        };
        if self
            .overrides
            .get(&mime)
            .is_some_and(|id| self.disabled.contains(id))
        {
            return true;
        }

        let id = match crate::fold::file_type::classify(path, head) {
            crate::fold::file_type::FileType::Pdf => "pdf",
            crate::fold::file_type::FileType::Video => "video",
            _ => return false,
        };
        self.disabled.iter().any(|s| s == id)
    }
}
fn helper_path(id: &str) -> PathBuf {
    match crate::bundled_preview::executable(id) {
        Ok(Some(path)) => return path,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!("Bundled preview unavailable: {error:#}");
            return PathBuf::from("/nonexistent/starfold-bundled-preview");
        }
    }

    // Never search the browsed directory or use its executables.
    {
        let executable = std::env::current_exe().unwrap_or_default();
        let directory = executable.parent().unwrap_or(Path::new("/"));
        #[cfg(test)]
        let directory = if directory.file_name().is_some_and(|name| name == "deps") {
            directory.parent().unwrap_or(directory)
        } else {
            directory
        };
        directory.join(format!("starfold-preview-{id}"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    pub provider: String,
    pub revision: String,
    pub session: u64,
    pub sequence: u64,
    pub keys: Vec<String>,
    pub actions: Vec<Action>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub sequence: u64,
    pub action: wire::MediaAction,
}
struct Packet {
    envelope: wire::Envelope<wire::Reply>,
    bytes: Vec<u8>,
}
pub struct Session {
    child: Child,
    input: Option<crossbeam_channel::Sender<wire::Envelope<wire::Request>>>,
    replies: crossbeam_channel::Receiver<anyhow::Result<Packet>>,
    threads: Vec<std::thread::JoinHandle<()>>,
    session: u64,
    sequence: u64,
    provider: String,
    revision: String,
    image: Option<std::sync::Arc<starkit::image::RgbaImage>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        // Writers never block the UI or cancellation thread on a full pipe.
        self.input = None;
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
impl Session {
    pub fn spawn(provider: &Provider) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !provider.id.is_empty() && provider.command.first().is_some_and(|s| !s.is_empty()),
            "Invalid preview provider command"
        );
        let mut command = Command::new(&provider.command[0]);
        command.args(&provider.command[1..]).process_group(0);
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                crate::fold::process::limit_memory();
                Ok(())
            });
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (input, requests) = crossbeam_channel::bounded::<wire::Envelope<wire::Request>>(2);
        let writer = std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                if wire::write_frame(&mut stdin, &request, &[]).is_err() {
                    break;
                }
            }
        });
        let (tx, replies) = crossbeam_channel::bounded(2);
        let reader = std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let packet = match wire::read_frame(&mut stdout) {
                    Ok(Some((envelope, bytes))) => Ok(Packet { envelope, bytes }),
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.try_send(Err(e));
                        break;
                    }
                };
                if tx.try_send(packet).is_err() {
                    break;
                }
            }
        });
        static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            child,
            input: Some(input),
            replies,
            threads: vec![writer, reader],
            session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            sequence: 0,
            provider: provider.id.clone(),
            revision: String::new(),
            image: None,
        })
    }
    fn request(
        &mut self,
        message: wire::Request,
        generation: u64,
        cfg: &PreviewConfig,
        stale: &dyn Fn() -> bool,
    ) -> anyhow::Result<Packet> {
        self.sequence = self.sequence.wrapping_add(1);
        self.input
            .as_ref()
            .unwrap()
            .try_send(wire::Envelope {
                session: self.session,
                generation,
                sequence: self.sequence,
                message,
            })
            .map_err(|_| anyhow::anyhow!("Preview request queue unavailable"))?;
        let deadline = Instant::now() + Duration::from_millis(cfg.timeout_ms);
        loop {
            anyhow::ensure!(!stale(), "Preview cancelled");
            anyhow::ensure!(Instant::now() < deadline, "Preview timed out");
            match self.replies.recv_timeout(Duration::from_millis(10)) {
                Ok(packet) => {
                    let packet = packet?;
                    anyhow::ensure!(
                        packet.envelope.session == self.session
                            && packet.envelope.sequence == self.sequence
                            && packet.envelope.generation == generation,
                        "Invalid preview reply scope"
                    );
                    if let wire::Reply::Error { message } = packet.envelope.message {
                        anyhow::bail!("{}", super::model::clean(&message, 512));
                    }
                    return Ok(packet);
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Preview extension stopped")
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    pub fn open(
        &mut self,
        path: &Path,
        generation: u64,
        viewport: wire::Viewport,
        cfg: &PreviewConfig,
        stale: &dyn Fn() -> bool,
    ) -> anyhow::Result<Preview> {
        let hello = self.request(
            wire::Request::Hello {
                version: wire::VERSION,
                capabilities: vec![
                    "documents".into(),
                    "raster".into(),
                    "surfaces".into(),
                    "media".into(),
                ],
            },
            generation,
            cfg,
            stale,
        )?;
        match hello.envelope.message {
            wire::Reply::Hello {
                version, revision, ..
            } if version == wire::VERSION && hello.bytes.is_empty() => {
                self.revision = super::model::clean(&revision, 128)
            }
            _ => anyhow::bail!("Incompatible preview extension"),
        }
        let packet = self.request(
            wire::Request::Open {
                path: path.as_os_str().as_bytes().to_vec(),
                limits: wire::Limits {
                    text_bytes: cfg.pdf_page_bytes,
                    cache_bytes: cfg.cache_bytes,
                    image_dimension: cfg.max_image_dimension.min(4096),
                },
                viewport,
            },
            generation,
            cfg,
            stale,
        )?;
        self.content(packet, path, cfg)
    }
    pub fn input(
        &mut self,
        path: &Path,
        generation: u64,
        input: wire::Input,
        cfg: &PreviewConfig,
        stale: &dyn Fn() -> bool,
    ) -> anyhow::Result<Preview> {
        let packet = self.request(wire::Request::Input { input }, generation, cfg, stale)?;
        self.content(packet, path, cfg)
    }
    fn content(
        &mut self,
        packet: Packet,
        path: &Path,
        cfg: &PreviewConfig,
    ) -> anyhow::Result<Preview> {
        let wire::Reply::Content { presentation: p } = packet.envelope.message else {
            anyhow::bail!("Expected preview content");
        };
        p.validate(&packet.bytes)?;
        if let Some(r) = &p.raster {
            anyhow::ensure!(
                r.width <= cfg.max_image_dimension && r.height <= cfg.max_image_dimension,
                "Raster exceeds configured limit"
            );
        }
        let image = if let Some(r) = &p.raster {
            if let Some(image) = self.image.as_ref().filter(|image| {
                image.width() == r.width
                    && image.height() == r.height
                    && image.as_raw() == &packet.bytes
            }) {
                Some(image.clone())
            } else {
                Some(std::sync::Arc::new(
                    starkit::image::RgbaImage::from_raw(r.width, r.height, packet.bytes)
                        .ok_or_else(|| anyhow::anyhow!("Invalid preview image"))?,
                ))
            }
        } else {
            None
        };
        self.image = image.clone();
        let info = Info {
            provider: self.provider.clone(),
            revision: self.revision.clone(),
            session: self.session,
            sequence: self.sequence,
            keys: p
                .keys
                .into_iter()
                .map(|s| super::model::clean(&s, 64))
                .collect(),
            actions: p
                .actions
                .into_iter()
                .map(|action| Action {
                    sequence: self.sequence,
                    action,
                })
                .collect(),
        };
        #[cfg(feature = "media")]
        if let Some(media) = p.media {
            let pixels = image.ok_or_else(|| anyhow::anyhow!("Video extension omitted poster"))?;
            return Ok(Preview::Video {
                path: path.to_owned(),
                poster: starkit::media::Poster {
                    pixels,
                    duration: media.duration,
                    width: media.width,
                    height: media.height,
                    audio: media.audio,
                },
                extension: Some(info),
            });
        }
        let mut d = Document::new(super::model::clean(&p.kind, 128));
        for (label, value) in p.fields {
            d.field(label, value);
        }
        if !p.pages.is_empty() {
            d.content = super::model::Content::Pages(
                p.pages
                    .into_iter()
                    .map(|p| Page {
                        number: p.number,
                        text: super::model::clean(&p.text, cfg.pdf_page_bytes),
                        truncated: p.truncated,
                    })
                    .collect(),
            );
        }
        d.total_pages = p.total_pages;
        d.next_page = p.next_page;
        d.notice = p.notice.map(|s| super::model::clean(&s, 512));
        d.image = image;
        d.image_page = p.raster.map(|r| r.page);
        d.surface = p.surface.map(Box::new);
        d.extension = Some(Box::new(info));
        Ok(Preview::Document(d))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_override_and_disable() {
        let mut registry = Registry::default();
        registry.providers.push(Provider {
            id: "custom".into(),
            command: vec!["/opt/helper".into()],
            extensions: vec!["xyz".into()],
            mime_types: vec![],
        });
        assert_eq!(
            registry.select(Path::new("test.xyz"), &[]).unwrap().id,
            "custom"
        );
        registry
            .overrides
            .insert("application/pdf".into(), "custom".into());
        assert_eq!(
            registry.select(Path::new("test.pdf"), b"%PDF-").unwrap().id,
            "custom"
        );
        registry.disabled.push("custom".into());
        assert!(registry.select(Path::new("test.pdf"), b"%PDF-").is_none());
    }
    proptest::proptest! {
        #[test]
        fn arbitrary_paths_and_heads_never_panic(path in proptest::collection::vec(proptest::prelude::any::<u8>(),0..200), head in proptest::collection::vec(proptest::prelude::any::<u8>(),0..512)) {
            use std::os::unix::ffi::OsStringExt;
            let path = PathBuf::from(std::ffi::OsString::from_vec(path));
            let registry = Registry::default();
            let _ = registry.select(&path,&head); let _ = registry.disables_builtin(&path,&head);
        }
    }
    #[test]
    fn cancellation_reaps_a_process_group_with_a_blocked_writer() {
        let provider = Provider {
            id: "stall".into(),
            command: vec!["sh".into(), "-c".into(), "sleep 30 & wait".into()],
            extensions: vec![],
            mime_types: vec![],
        };
        let mut session = Session::spawn(&provider).unwrap();
        let start = Instant::now();
        let cfg = PreviewConfig {
            timeout_ms: 30,
            ..Default::default()
        };
        let result = session.request(
            wire::Request::Open {
                path: vec![b'x'; 200_000],
                limits: wire::Limits {
                    text_bytes: 4096,
                    cache_bytes: 4096,
                    image_dimension: 4096,
                },
                viewport: Default::default(),
            },
            5,
            &cfg,
            &|| start.elapsed() > Duration::from_millis(20),
        );
        assert!(result.err().unwrap().to_string().contains("cancelled"));
        drop(session);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn stalled_child_is_cancelled_without_blocking_writer() {
        let provider = Provider {
            id: "stall".into(),
            command: vec!["sh".into(), "-c".into(), "exec sleep 30".into()],
            extensions: vec![],
            mime_types: vec![],
        };
        let mut session = Session::spawn(&provider).unwrap();
        let start = Instant::now();
        let result = session.open(
            Path::new("x"),
            2,
            Default::default(),
            &PreviewConfig {
                timeout_ms: 30,
                ..Default::default()
            },
            &|| false,
        );
        assert!(result.unwrap_err().to_string().contains("timed out"));
        drop(session);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
