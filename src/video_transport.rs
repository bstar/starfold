//! Bounded render-only STAR/AMP helper. AMP owns artwork, layout and hit testing.
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use starkit::native_surface::Surface;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Request {
    pub cells: Option<[u16; 2]>,
    pub width: u16,
    pub height: u16,
    pub theme: crate::audio_embed::Palette,
    pub playing: bool,
    pub paused: bool,
    pub volume: f32,
    pub pointer: Option<[u16; 2]>,
    pub movie: bool,
    pub picker: bool,
    pub entries: Vec<String>,
    pub selected: usize,
    pub title: String,
    pub font: u16,
}
#[derive(Clone, Deserialize)]
pub struct CellTransport {
    pub columns: u16,
    pub rows: u16,
    pub cells: Vec<crate::audio_embed::Cell>,
}
#[derive(Deserialize)]
struct Response {
    surface: Option<Surface>,
    cells: Option<CellTransport>,
    action: Option<String>,
    value: Option<f32>,
}
#[derive(Default)]
struct Latest {
    frame: Option<(Request, Surface)>,
    cells: Option<CellTransport>,
    changed: bool,
    error: Option<String>,
}
pub struct Client {
    requests: Sender<Request>,
    actions: Receiver<(String, Option<f32>)>,
    latest: Arc<Mutex<Latest>>,
    requested: Option<Request>,
    quit: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Client {
    pub fn new() -> Self {
        let (requests, rx) = bounded::<Request>(8);
        let (tx, actions) = bounded(8);
        let latest = Arc::new(Mutex::new(Latest::default()));
        let quit = Arc::new(AtomicBool::new(false));
        let worker = {
            let latest = latest.clone();
            let quit = quit.clone();
            thread::Builder::new()
                .name("starfold-video-controls".into())
                .spawn(move || {
                    if let Err(error) = serve(rx, tx, &latest, &quit) {
                        let mut latest = latest.lock().unwrap();
                        latest.error = Some(format!("Video controls: {error:#}"));
                        latest.changed = true;
                    }
                })
                .expect("video control supervisor can start")
        };
        Self {
            requests,
            actions,
            latest,
            requested: None,
            quit,
            worker: Some(worker),
        }
    }
    pub fn render(&mut self, request: Request) -> Option<Surface> {
        if self.requested.as_ref() != Some(&request)
            && self.requests.try_send(request.clone()).is_ok()
        {
            self.requested = Some(request.clone());
        }
        self.latest
            .lock()
            .unwrap()
            .frame
            .as_ref()
            .filter(|(key, _)| key == &request)
            .map(|(_, frame)| frame.clone())
    }
    pub fn cells(&self, request: &Request) -> Option<CellTransport> {
        let latest = self.latest.lock().unwrap();
        latest.frame.as_ref().filter(|(key, _)| key == request)?;
        latest.cells.clone()
    }
    pub fn pointer(&self, mut request: Request, x: u16, y: u16) {
        request.pointer = Some([x, y]);
        let _ = self.requests.try_send(request);
    }
    pub fn poll(&self) -> (bool, Option<String>) {
        let mut latest = self.latest.lock().unwrap();
        (std::mem::take(&mut latest.changed), latest.error.take())
    }
    pub fn action(&self) -> Option<(String, Option<f32>)> {
        self.actions.try_recv().ok()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn serve(
    requests: Receiver<Request>,
    actions: Sender<(String, Option<f32>)>,
    latest: &Mutex<Latest>,
    quit: &AtomicBool,
) -> anyhow::Result<()> {
    // Spawn lazily: a selected poster alone never starts an audio player.
    let first = loop {
        if quit.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok(request) = requests.recv_timeout(Duration::from_millis(50)) {
            break request;
        }
    };
    let mut child = crate::bundled_amp::command()
        .args(["embed", "--stdio", "--transport"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, responses) = bounded(1);
    let reader = thread::spawn(move || {
        let mut output = BufReader::new(output);
        loop {
            let mut line = Vec::new();
            let result = (|| -> anyhow::Result<Response> {
                output.by_ref().take(65537).read_until(b'\n', &mut line)?;
                anyhow::ensure!(
                    line.last() == Some(&b'\n') && line.len() <= 65536,
                    "AMP transport response missing or oversized"
                );
                let response: Response = serde_json::from_slice(&line)?;
                if let Some(cells) = &response.cells {
                    let count = usize::from(cells.columns) * usize::from(cells.rows);
                    anyhow::ensure!(count <= 512 && count > 0 && cells.cells.len() == count
                        && cells.cells.iter().all(|c| c.symbol.len() <= 128 && starkit::wrap::width_of(&c.symbol) <= 2 && !c.symbol.chars().any(|c| c.is_control() || matches!(c,'\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))), "Invalid AMP transport cells");
                }
                if let Some(surface) = &response.surface {
                    surface.validate()?;
                }
                Ok(response)
            })();
            let failed = result.is_err();
            if tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    let result = (|| -> anyhow::Result<()> {
        let mut next = Some(first);
        while !quit.load(Ordering::Relaxed) {
            let request = match next
                .take()
                .or_else(|| requests.recv_timeout(Duration::from_millis(50)).ok())
            {
                Some(r) => r,
                None => continue,
            };
            serde_json::to_writer(&mut input, &request)?;
            input.write_all(b"\n")?;
            input.flush()?;
            let deadline = Instant::now() + Duration::from_secs(3);
            let response = loop {
                if quit.load(Ordering::Relaxed) {
                    return Ok(());
                }
                anyhow::ensure!(Instant::now() < deadline, "AMP transport timed out");
                match responses.recv_timeout(Duration::from_millis(50)) {
                    Ok(response) => break response?,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        anyhow::bail!("AMP transport closed")
                    }
                    Err(_) => {}
                }
            };
            if let Some(surface) = response.surface {
                let mut latest = latest.lock().unwrap();
                latest.cells = response.cells;
                latest.frame = Some((request, surface));
                latest.changed = true;
            } else if let Some(action) = response.action {
                let _ = actions.try_send((action, response.value));
            }
        }
        Ok(())
    })();
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    drop(responses);
    let _ = reader.join();
    result
}
