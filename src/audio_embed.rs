//! A supervised JSONL client for STAR/AMP's terminal embed mode.
//!
//! The UI owns a `Client`, but never waits for the player: one supervisor
//! thread owns the child, with small reader and writer threads around its
//! pipes. Frames replace one another in a single slot rather than queueing.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use serde::{Deserialize, Serialize};

const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_TIMEOUT: Duration = Duration::from_millis(450);
const TICK: Duration = Duration::from_millis(15);
const MAX_LINE: usize = 2 * 1024 * 1024;
const MAX_HOST_LINE: usize = 1024 * 1024;
const MAX_EVENTS: usize = 32;
const MAX_CELLS: usize = 150_000;
const MAX_TRANSPORT_IMAGES: usize = 5;
const MAX_IMAGE_PIXELS: usize = 65_536;
const MAX_RGBA_TOTAL: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
    pub selected: [u8; 3],
    pub border: [u8; 3],
    pub error: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GraphicsConfig {
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presentation {
    pub generation: u64,
    pub width: u16,
    pub height: u16,
    pub focused: bool,
    pub theme: Palette,
    pub graphics: Option<GraphicsConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Cell {
    pub symbol: String,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub modifiers: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub generation: u64,
    pub width: u16,
    pub height: u16,
    pub cells: Vec<Cell>,
    pub images: Vec<TransportImage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TransportImage {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub playing: bool,
    pub paused: bool,
    pub title: String,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Accepted {
        generation: u64,
    },
    Fallback {
        generation: u64,
        path: PathBuf,
        reason: String,
    },
    Status {
        generation: u64,
        status: Status,
    },
    Error {
        generation: u64,
        message: String,
    },
    Notice {
        generation: u64,
        message: String,
    },
    Stopped {
        generation: u64,
    },
}

impl Event {
    fn generation(&self) -> u64 {
        match self {
            Self::Accepted { generation }
            | Self::Fallback { generation, .. }
            | Self::Status { generation, .. }
            | Self::Error { generation, .. }
            | Self::Notice { generation, .. }
            | Self::Stopped { generation } => *generation,
        }
    }
}

#[derive(Default)]
struct Shared {
    serial: u64,
    request_generation: u64,
    frame_generation: u64,
    extensions: Option<Vec<String>>,
    transport_images: bool,
    player_styles: bool,
    frame: Option<Frame>,
    events: VecDeque<Event>,
}

fn lock(shared: &Arc<Mutex<Shared>>) -> std::sync::MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn push_event(shared: &Arc<Mutex<Shared>>, event: Event) {
    let event = match event {
        Event::Error {
            generation,
            message,
        } => Event::Error {
            generation,
            message: safe_message(&message),
        },
        Event::Notice {
            generation,
            message,
        } => Event::Notice {
            generation,
            message: safe_message(&message),
        },
        Event::Fallback {
            generation,
            path,
            reason,
        } => Event::Fallback {
            generation,
            path,
            reason: safe_message(&reason),
        },
        Event::Status {
            generation,
            mut status,
        } => {
            status.title = safe_message(&status.title);
            Event::Status { generation, status }
        }
        other => other,
    };
    let mut data = lock(shared);
    enqueue_event(&mut data, event);
}

fn enqueue_event(data: &mut Shared, event: Event) {
    if event.generation() != data.request_generation {
        return;
    }
    if data.events.len() >= MAX_EVENTS {
        if let Some(i) = data
            .events
            .iter()
            .position(|e| matches!(e, Event::Status { .. }))
        {
            data.events.remove(i);
        } else {
            data.events.pop_front();
        }
    }
    data.events.push_back(event);
}

fn safe_message(message: &str) -> String {
    message
        .chars()
        .filter(|c| !c.is_control())
        .take(1024)
        .collect()
}

#[derive(Debug, Clone)]
struct Activation {
    serial: u64,
    selected: PathBuf,
    candidates: Vec<PathBuf>,
    token: u64,
    presentation: Presentation,
}

enum Request {
    Activate(Activation),
    KeepPlaying {
        serial: u64,
        token: u64,
        presentation: Presentation,
    },
    Configure {
        serial: u64,
        presentation: Presentation,
    },
    Control {
        serial: u64,
        action: String,
        value: Option<f64>,
    },
    Pointer {
        serial: u64,
        x: u16,
        y: u16,
        button: String,
    },
    Quit,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostMessage<'a> {
    Configure {
        generation: u64,
        width: u16,
        height: u16,
        focused: bool,
        theme: &'a Palette,
        #[serde(skip_serializing_if = "Option::is_none")]
        graphics: Option<&'a GraphicsConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        profile: Option<&'a str>,
    },
    Play {
        generation: u64,
        paths: &'a [String],
        index: usize,
    },
    Control {
        action: &'a str,
        value: Option<f64>,
    },
    Pointer {
        x: u16,
        y: u16,
        button: &'a str,
    },
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChildMessage {
    Hello {
        protocol: u32,
        extensions: Vec<String>,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    Frame {
        generation: u64,
        width: u16,
        height: u16,
        cells: Vec<Cell>,
        #[serde(default)]
        images: Vec<TransportImage>,
    },
    Status {
        playing: bool,
        paused: bool,
        title: String,
        #[serde(default)]
        path: Option<PathBuf>,
    },
    Error {
        message: String,
    },
    Notice {
        message: String,
    },
    Stopped,
}

enum IoEvent {
    Child(ChildMessage),
    Written(u64),
    Failure(String),
    Eof,
}

struct WriteJob {
    sequence: u64,
    bytes: Vec<u8>,
}

struct Running {
    child: Child,
    writer: Sender<WriteJob>,
    io: Receiver<IoEvent>,
    frames: Arc<Mutex<Option<ChildMessage>>>,
    activation: Activation,
    hello_deadline: Instant,
    hello: bool,
    accepted: bool,
    pending_play: Option<u64>,
    next_sequence: u64,
    extensions: Vec<String>,
    transport_images: bool,
    player_styles: bool,
}

/// A nonblocking owner for one embedded player. `new` starts only a cheap
/// supervisor; the `staramp` process is spawned on the first activation.
pub struct Client {
    requests: Sender<Request>,
    shared: Arc<Mutex<Shared>>,
    quitting: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    supervisor: Option<JoinHandle<()>>,
}

impl Client {
    pub fn new() -> Self {
        Self::with_executable(PathBuf::from("staramp"))
    }

    /// Also useful for a packaged player path.
    pub fn with_executable(executable: PathBuf) -> Self {
        let mut command = Command::new(executable);
        command.args(["embed", "--stdio"]);
        Self::with_command(command)
    }

    fn with_command(command: Command) -> Self {
        let (requests, receiver) = bounded(32);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let quitting = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let supervisor = {
            let shared = Arc::clone(&shared);
            let quitting = Arc::clone(&quitting);
            let stopping = Arc::clone(&stopping);
            thread::Builder::new()
                .name("starfold-audio".into())
                .spawn(move || supervise(command, receiver, shared, quitting, stopping))
                .expect("the audio supervisor thread can start")
        };
        Self {
            requests,
            shared,
            quitting,
            stopping,
            supervisor: Some(supervisor),
        }
    }

    /// `None` until the helper advertises its authoritative extension list.
    /// `false` then lets the UI use its external opener without disturbing
    /// current audio.
    pub fn supports(&self, path: &Path) -> Option<bool> {
        let data = lock(&self.shared);
        data.extensions
            .as_ref()
            .map(|known| supports_extension(path, known))
    }

    pub fn transport_images_available(&self) -> bool {
        lock(&self.shared).transport_images
    }

    pub fn player_styles_available(&self) -> bool {
        lock(&self.shared).player_styles
    }

    pub fn activate(
        &mut self,
        selected: PathBuf,
        candidates: Vec<PathBuf>,
        presentation: Presentation,
    ) -> Result<(), String> {
        let token = presentation.generation;
        let mut data = lock(&self.shared);
        let serial = data.serial.wrapping_add(1);
        let unsupported = if !selected.is_absolute() {
            Some("audio path is not absolute".to_string())
        } else if selected.to_str().is_none() {
            Some("audio path is not UTF-8".to_string())
        } else if data
            .extensions
            .as_ref()
            .is_some_and(|known| !supports_extension(&selected, known))
        {
            Some("the player does not support this file type".to_string())
        } else {
            None
        };
        let request = if unsupported.is_some() {
            Request::KeepPlaying {
                serial,
                token,
                presentation: presentation.clone(),
            }
        } else {
            Request::Activate(Activation {
                serial,
                selected: selected.clone(),
                candidates,
                token,
                presentation: presentation.clone(),
            })
        };
        self.send(request)?;
        data.serial = serial;
        data.request_generation = token;
        data.frame_generation = presentation.generation;
        data.frame = None;
        data.events.clear();
        if let Some(reason) = unsupported {
            data.events.push_back(Event::Fallback {
                generation: token,
                path: selected,
                reason,
            });
        }
        Ok(())
    }

    pub fn configure(&mut self, presentation: Presentation) -> Result<(), String> {
        let mut data = lock(&self.shared);
        self.send(Request::Configure {
            serial: data.serial,
            presentation: presentation.clone(),
        })?;
        if data.frame_generation != presentation.generation {
            data.frame = None;
        }
        data.frame_generation = presentation.generation;
        Ok(())
    }

    pub fn control(&self, action: &str, value: Option<f64>) -> Result<(), String> {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err("audio control value must be finite".into());
        }
        let data = lock(&self.shared);
        self.send(Request::Control {
            serial: data.serial,
            action: action.into(),
            value,
        })
    }

    pub fn pointer(&self, x: u16, y: u16, button: &str) -> Result<(), String> {
        let data = lock(&self.shared);
        self.send(Request::Pointer {
            serial: data.serial,
            x,
            y,
            button: button.into(),
        })
    }

    pub fn stop(&mut self) -> Result<(), String> {
        {
            let mut data = lock(&self.shared);
            data.serial = data.serial.wrapping_add(1);
            data.request_generation = data.request_generation.wrapping_add(1);
            data.frame = None;
            data.events.clear();
        }
        self.stopping.store(true, Ordering::Release);
        // The flag is the guarantee: it is observed even if this bounded
        // command queue is full of earlier layout updates.
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        lock(&self.shared).events.drain(..).collect()
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        lock(&self.shared).frame.take()
    }

    /// A copy for callers that retain no frame themselves.
    pub fn latest_frame(&self) -> Option<Frame> {
        lock(&self.shared).frame.clone()
    }

    fn send(&self, request: Request) -> Result<(), String> {
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                TrySendError::Full(_) => "audio command queue is full".to_string(),
                TrySendError::Disconnected(_) => "audio supervisor has stopped".to_string(),
            })
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.quitting.store(true, Ordering::Release);
        let _ = self.requests.try_send(Request::Quit);
        if let Some(thread) = self.supervisor.take() {
            let _ = thread.join();
        }
    }
}

fn supervise(
    mut command: Command,
    requests: Receiver<Request>,
    shared: Arc<Mutex<Shared>>,
    quitting: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
) {
    let mut running: Option<Running> = None;
    loop {
        if quitting.load(Ordering::Acquire) {
            break;
        }
        if stopping.swap(false, Ordering::AcqRel) {
            if let Some(player) = running.take() {
                stop_child(player);
            }
        }
        match requests.recv_timeout(TICK) {
            Ok(Request::Quit) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Ok(Request::Activate(activation)) => {
                if activation.serial != lock(&shared).serial {
                    continue;
                }
                let mut rejected = false;
                if let Some(player) = &mut running {
                    if player.hello {
                        player.activation = activation;
                        player.accepted = false;
                        player.pending_play = None;
                        if let Err(reason) = queue_play(player, &shared) {
                            push_event(
                                &shared,
                                Event::Error {
                                    generation: player.activation.token,
                                    message: reason,
                                },
                            );
                            rejected = true;
                        }
                    } else {
                        player.activation = activation;
                    }
                } else {
                    match spawn_child(&mut command, activation) {
                        Ok(player) => running = Some(player),
                        Err((token, path, reason)) => push_event(
                            &shared,
                            Event::Fallback {
                                generation: token,
                                path,
                                reason,
                            },
                        ),
                    }
                }
                if rejected {
                    if let Some(player) = running.take() {
                        stop_child(player);
                    }
                }
            }
            Ok(Request::KeepPlaying {
                serial,
                token,
                presentation,
            }) => {
                if serial != lock(&shared).serial {
                    continue;
                }
                if let Some(player) = &mut running {
                    player.activation.token = token;
                    player.activation.presentation = presentation;
                    if player.hello {
                        let _ = send_configure(player);
                    }
                }
            }
            Ok(Request::Configure {
                serial,
                presentation,
            }) => {
                if serial != lock(&shared).serial {
                    continue;
                }
                if let Some(player) = &mut running {
                    player.activation.presentation = presentation;
                    if player.hello {
                        if let Err(message) = send_configure(player) {
                            push_event(
                                &shared,
                                Event::Error {
                                    generation: player.activation.token,
                                    message,
                                },
                            );
                        }
                    }
                }
            }
            Ok(Request::Control {
                serial,
                action,
                value,
            }) => {
                if serial != lock(&shared).serial {
                    continue;
                }
                if let Some(player) = &mut running {
                    if player.hello {
                        if let Err(message) = queue_message(
                            player,
                            &HostMessage::Control {
                                action: &action,
                                value,
                            },
                        ) {
                            push_event(
                                &shared,
                                Event::Error {
                                    generation: player.activation.token,
                                    message,
                                },
                            );
                        }
                    }
                }
            }
            Ok(Request::Pointer {
                serial,
                x,
                y,
                button,
            }) => {
                if serial != lock(&shared).serial {
                    continue;
                }
                if let Some(player) = &mut running {
                    if player.hello {
                        if let Err(message) = queue_message(
                            player,
                            &HostMessage::Pointer {
                                x,
                                y,
                                button: &button,
                            },
                        ) {
                            push_event(
                                &shared,
                                Event::Error {
                                    generation: player.activation.token,
                                    message,
                                },
                            );
                        }
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if let Some(player) = &mut running {
            let mut close = false;
            for _ in 0..16 {
                let Ok(event) = player.io.try_recv() else {
                    break;
                };
                if handle_io(player, event, &shared) {
                    close = true;
                    break;
                }
            }
            if !close {
                let frame = player
                    .frames
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some(frame) = frame {
                    close = handle_io(player, IoEvent::Child(frame), &shared);
                }
            }
            if !player.hello && Instant::now() >= player.hello_deadline {
                push_event(
                    &shared,
                    Event::Fallback {
                        generation: player.activation.token,
                        path: player.activation.selected.clone(),
                        reason: "STAR/AMP did not complete its embed handshake".into(),
                    },
                );
                close = true;
            }
            match player.child.try_wait() {
                Ok(Some(status)) if !close => {
                    let event = if player.accepted {
                        Event::Error {
                            generation: player.activation.token,
                            message: format!("STAR/AMP exited: {status}"),
                        }
                    } else {
                        Event::Fallback {
                            generation: player.activation.token,
                            path: player.activation.selected.clone(),
                            reason: format!("STAR/AMP exited before handshake: {status}"),
                        }
                    };
                    push_event(&shared, event);
                    close = true;
                }
                Err(error) if !close => {
                    push_event(
                        &shared,
                        Event::Error {
                            generation: player.activation.token,
                            message: format!("could not inspect STAR/AMP: {error}"),
                        },
                    );
                    close = true;
                }
                _ => {}
            }
            if close {
                if let Some(player) = running.take() {
                    stop_child(player);
                }
            }
        }
    }
    if let Some(player) = running.take() {
        stop_child(player);
    }
}

fn spawn_child(
    command: &mut Command,
    activation: Activation,
) -> Result<Running, (u64, PathBuf, String)> {
    let token = activation.token;
    let path = activation.selected.clone();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            (
                token,
                path.clone(),
                format!("could not start STAR/AMP: {error}"),
            )
        })?;
    let Some(stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err((token, path, "STAR/AMP has no input pipe".into()));
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err((token, path, "STAR/AMP has no output pipe".into()));
    };
    let (io_tx, io) = bounded(16);
    let (writer, writes) = bounded(8);
    let frames = Arc::new(Mutex::new(None));
    spawn_reader(stdout, io_tx.clone(), Arc::clone(&frames));
    spawn_writer(stdin, writes, io_tx);
    Ok(Running {
        child,
        writer,
        io,
        frames,
        activation,
        hello_deadline: Instant::now() + HELLO_TIMEOUT,
        hello: false,
        accepted: false,
        pending_play: None,
        next_sequence: 1,
        extensions: Vec::new(),
        transport_images: false,
        player_styles: false,
    })
}

fn spawn_reader(
    stdout: impl Read + Send + 'static,
    events: Sender<IoEvent>,
    frames: Arc<Mutex<Option<ChildMessage>>>,
) {
    thread::Builder::new()
        .name("starfold-audio-read".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let read = reader
                    .by_ref()
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => {
                        let _ = events.send_timeout(IoEvent::Eof, Duration::from_millis(100));
                        break;
                    }
                    Ok(_) if bytes.len() > MAX_LINE || !bytes.ends_with(b"\n") => {
                        let _ = events.send_timeout(
                            IoEvent::Failure(
                                "STAR/AMP sent an oversized or unterminated message".into(),
                            ),
                            Duration::from_millis(100),
                        );
                        break;
                    }
                    Ok(_) => match serde_json::from_slice::<ChildMessage>(&bytes) {
                        Ok(ChildMessage::Frame {
                            generation,
                            width,
                            height,
                            cells,
                            images,
                        }) => {
                            *frames
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                Some(ChildMessage::Frame {
                                    generation,
                                    width,
                                    height,
                                    cells,
                                    images,
                                });
                        }
                        Ok(message) => {
                            if events
                                .send_timeout(IoEvent::Child(message), Duration::from_millis(250))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = events.send_timeout(
                                IoEvent::Failure(format!("invalid STAR/AMP message: {error}")),
                                Duration::from_millis(100),
                            );
                            break;
                        }
                    },
                    Err(error) => {
                        let _ = events.send_timeout(
                            IoEvent::Failure(format!("could not read STAR/AMP: {error}")),
                            Duration::from_millis(100),
                        );
                        break;
                    }
                }
            }
        })
        .expect("audio reader thread can start");
}

fn spawn_writer(mut stdin: ChildStdin, writes: Receiver<WriteJob>, events: Sender<IoEvent>) {
    thread::Builder::new()
        .name("starfold-audio-write".into())
        .spawn(move || {
            for job in writes {
                if let Err(error) = stdin.write_all(&job.bytes).and_then(|_| stdin.flush()) {
                    let _ = events.send_timeout(
                        IoEvent::Failure(format!("could not write STAR/AMP: {error}")),
                        Duration::from_millis(100),
                    );
                    break;
                }
                if events
                    .send_timeout(IoEvent::Written(job.sequence), Duration::from_millis(250))
                    .is_err()
                {
                    break;
                }
            }
        })
        .expect("audio writer thread can start");
}

fn queue_message(player: &mut Running, message: &HostMessage<'_>) -> Result<u64, String> {
    let mut bytes =
        serde_json::to_vec(message).map_err(|e| format!("could not encode audio command: {e}"))?;
    if bytes.len() + 1 > MAX_HOST_LINE {
        return Err("audio command exceeds the message limit".into());
    }
    bytes.push(b'\n');
    let sequence = player.next_sequence;
    player.next_sequence = player.next_sequence.wrapping_add(1);
    player
        .writer
        .try_send(WriteJob { sequence, bytes })
        .map_err(|error| match error {
            TrySendError::Full(_) => "STAR/AMP input queue is full".to_string(),
            TrySendError::Disconnected(_) => "STAR/AMP input has closed".to_string(),
        })?;
    Ok(sequence)
}

fn send_configure(player: &mut Running) -> Result<u64, String> {
    let p = player.activation.presentation.clone();
    queue_message(
        player,
        &HostMessage::Configure {
            generation: p.generation,
            width: p.width,
            height: p.height,
            focused: p.focused,
            theme: &p.theme,
            graphics: if player.transport_images {
                p.graphics.as_ref().filter(|config| valid_graphics(config))
            } else {
                None
            },
            profile: player.player_styles.then_some("starfold"),
        },
    )
}

fn queue_play(player: &mut Running, shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
    let selected = &player.activation.selected;
    if !supports_extension(selected, &player.extensions) {
        push_event(
            shared,
            Event::Fallback {
                generation: player.activation.token,
                path: selected.clone(),
                reason: "the player does not support this file type".into(),
            },
        );
        return Ok(());
    }
    let mut paths = Vec::new();
    let mut index = None;
    for candidate in &player.activation.candidates {
        if !candidate.is_absolute() {
            return Err("playlist contains a relative path".into());
        }
        let Some(path) = candidate.to_str() else {
            return Err("playlist contains a non-UTF-8 path".into());
        };
        if !supports_extension(candidate, &player.extensions) {
            continue;
        }
        if candidate == selected && index.is_none() {
            index = Some(paths.len());
        }
        paths.push(path.to_string());
    }
    if index.is_none() {
        let Some(path) = selected.to_str() else {
            push_event(
                shared,
                Event::Fallback {
                    generation: player.activation.token,
                    path: selected.clone(),
                    reason: "selected audio path is not UTF-8".into(),
                },
            );
            return Ok(());
        };
        index = Some(0);
        paths.insert(0, path.to_owned());
    }
    send_configure(player)?;
    let sequence = queue_message(
        player,
        &HostMessage::Play {
            generation: player.activation.presentation.generation,
            paths: &paths,
            index: index.expect("selected path inserted"),
        },
    )?;
    player.pending_play = Some(sequence);
    Ok(())
}

fn handle_io(player: &mut Running, event: IoEvent, shared: &Arc<Mutex<Shared>>) -> bool {
    match event {
        IoEvent::Child(ChildMessage::Hello {
            protocol,
            extensions,
            capabilities,
        }) => {
            if player.hello || protocol != 1 {
                push_event(
                    shared,
                    Event::Fallback {
                        generation: player.activation.token,
                        path: player.activation.selected.clone(),
                        reason: "STAR/AMP has an incompatible embed protocol".into(),
                    },
                );
                return true;
            }
            player.hello = true;
            player.extensions = extensions
                .into_iter()
                .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                .collect();
            player.transport_images = capabilities.iter().any(|value| value == "transport_images");
            player.player_styles = capabilities.iter().any(|value| value == "player_styles");
            {
                let mut data = lock(shared);
                data.extensions = Some(player.extensions.clone());
                data.transport_images = player.transport_images;
                data.player_styles = player.player_styles;
            }
            if let Err(message) = queue_play(player, shared) {
                push_event(
                    shared,
                    Event::Error {
                        generation: player.activation.token,
                        message,
                    },
                );
                return true;
            }
        }
        IoEvent::Child(ChildMessage::Frame {
            generation,
            width,
            height,
            cells,
            images,
        }) => {
            if player.accepted {
                let frame = Frame {
                    generation,
                    width,
                    height,
                    cells,
                    images,
                };
                // A resize or button-mode change can overtake a frame in the
                // reader slot. Its images belong to the old negotiation, so
                // discard it before applying the current graphics limits.
                let current = |data: &Shared| {
                    data.request_generation == player.activation.token
                        && data.frame_generation == frame.generation
                        && player.activation.presentation.generation == frame.generation
                };
                if !current(&lock(shared)) {
                    return false;
                }
                let graphics = player
                    .transport_images
                    .then_some(player.activation.presentation.graphics)
                    .flatten();
                let graphics = graphics.as_ref().filter(|config| valid_graphics(config));
                let validation = validate_frame(&frame, graphics);
                let mut data = lock(shared);
                if !current(&data) {
                    return false;
                }
                match validation {
                    Err(message) => {
                        enqueue_event(
                            &mut data,
                            Event::Error {
                                generation: player.activation.token,
                                message,
                            },
                        );
                        return true;
                    }
                    Ok(()) => data.frame = Some(frame),
                }
            }
        }
        IoEvent::Child(ChildMessage::Status {
            playing,
            paused,
            title,
            path,
        }) => {
            if player.accepted {
                push_event(
                    shared,
                    Event::Status {
                        generation: player.activation.token,
                        status: Status {
                            playing,
                            paused,
                            title,
                            path,
                        },
                    },
                );
            }
        }
        IoEvent::Child(ChildMessage::Error { message }) => {
            push_event(
                shared,
                Event::Error {
                    generation: player.activation.token,
                    message,
                },
            );
        }
        IoEvent::Child(ChildMessage::Notice { message }) => {
            push_event(
                shared,
                Event::Notice {
                    generation: player.activation.token,
                    message,
                },
            );
        }
        IoEvent::Child(ChildMessage::Stopped) => {
            push_event(
                shared,
                Event::Stopped {
                    generation: player.activation.token,
                },
            );
        }
        IoEvent::Written(sequence) => {
            if player.pending_play == Some(sequence) {
                player.pending_play = None;
                player.accepted = true;
                push_event(
                    shared,
                    Event::Accepted {
                        generation: player.activation.token,
                    },
                );
            }
        }
        IoEvent::Failure(message) => {
            let event = if player.accepted {
                Event::Error {
                    generation: player.activation.token,
                    message,
                }
            } else {
                Event::Fallback {
                    generation: player.activation.token,
                    path: player.activation.selected.clone(),
                    reason: message,
                }
            };
            push_event(shared, event);
            return true;
        }
        IoEvent::Eof => {
            let event = if player.accepted {
                Event::Error {
                    generation: player.activation.token,
                    message: "STAR/AMP closed its output".into(),
                }
            } else {
                Event::Fallback {
                    generation: player.activation.token,
                    path: player.activation.selected.clone(),
                    reason: "STAR/AMP closed before handshake".into(),
                }
            };
            push_event(shared, event);
            return true;
        }
    }
    false
}

fn stop_child(mut player: Running) {
    let _ = queue_message(&mut player, &HostMessage::Shutdown);
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match player.child.try_wait() {
            Ok(Some(_)) => return,
            _ if Instant::now() >= deadline => break,
            _ => thread::sleep(Duration::from_millis(10)),
        }
    }
    let _ = player.child.kill();
    let _ = player.child.wait();
}

fn supports_extension(path: &Path, extensions: &[String]) -> bool {
    let Some(extension) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    extensions
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

fn valid_graphics(config: &GraphicsConfig) -> bool {
    (1..=64).contains(&config.cell_width) && (1..=128).contains(&config.cell_height)
}

fn validate_frame(frame: &Frame, graphics: Option<&GraphicsConfig>) -> Result<(), String> {
    let count = usize::from(frame.width).saturating_mul(usize::from(frame.height));
    if frame.width == 0 || frame.height == 0 || count > MAX_CELLS || frame.cells.len() != count {
        return Err("STAR/AMP sent an invalid frame size".into());
    }
    for cell in &frame.cells {
        if cell.symbol.is_empty()
            || cell.symbol.len() > 64
            || cell.symbol.chars().any(char::is_control)
            || starkit::wrap::width_of(&cell.symbol) > 2
        {
            return Err("STAR/AMP sent an unsafe frame symbol".into());
        }
    }
    if frame.images.len() > MAX_TRANSPORT_IMAGES {
        return Err("STAR/AMP sent too many transport images".into());
    }
    if !frame.images.is_empty() && graphics.is_none() {
        return Err("STAR/AMP sent transport images without graphics negotiation".into());
    }
    if !frame.images.is_empty() && graphics.is_some_and(|config| !valid_graphics(config)) {
        return Err("STAR/AMP sent transport images for an invalid cell size".into());
    }
    let mut rgba_total = 0usize;
    for image in &frame.images {
        if image.width == 0
            || image.height == 0
            || image.pixel_width == 0
            || image.pixel_height == 0
        {
            return Err("STAR/AMP sent an empty transport image".into());
        }
        let x_end = u32::from(image.x) + u32::from(image.width);
        let y_end = u32::from(image.y) + u32::from(image.height);
        if x_end > u32::from(frame.width) || y_end > u32::from(frame.height) {
            return Err("STAR/AMP sent a transport image outside its frame".into());
        }
        let config = graphics.expect("checked nonempty images above");
        let target_width = u32::from(image.width) * u32::from(config.cell_width);
        let target_height = u32::from(image.height) * u32::from(config.cell_height);
        if u32::from(image.pixel_width) != target_width
            || u32::from(image.pixel_height) != target_height
        {
            return Err("STAR/AMP sent a transport image at the wrong pixel size".into());
        }
        let pixels = usize::from(image.pixel_width)
            .checked_mul(usize::from(image.pixel_height))
            .ok_or("STAR/AMP transport image dimensions overflow")?;
        if pixels > MAX_IMAGE_PIXELS {
            return Err("STAR/AMP sent an oversized transport image".into());
        }
        let bytes = pixels
            .checked_mul(4)
            .ok_or("STAR/AMP transport image size overflow")?;
        if image.rgba.len() != bytes {
            return Err("STAR/AMP sent an incomplete transport image".into());
        }
        rgba_total = rgba_total
            .checked_add(bytes)
            .ok_or("STAR/AMP transport image total overflow")?;
        if rgba_total > MAX_RGBA_TOTAL {
            return Err("STAR/AMP sent too much transport image data".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_frame(width: u16, height: u16) -> Frame {
        Frame {
            generation: 1,
            width,
            height,
            cells: vec![
                Cell {
                    symbol: " ".into(),
                    fg: [255; 3],
                    bg: [0; 3],
                    modifiers: 0,
                };
                usize::from(width) * usize::from(height)
            ],
            images: Vec::new(),
        }
    }

    fn image(x: u16, y: u16, width: u16, height: u16, config: GraphicsConfig) -> TransportImage {
        let pixel_width = width * config.cell_width;
        let pixel_height = height * config.cell_height;
        TransportImage {
            x,
            y,
            width,
            height,
            pixel_width,
            pixel_height,
            rgba: vec![0; usize::from(pixel_width) * usize::from(pixel_height) * 4],
        }
    }

    #[cfg(unix)]
    fn fake_helper(source: &str) -> Command {
        // Execute an installed shell, not a freshly written executable that
        // can race with concurrent process creation and fail with ETXTBSY.
        // The script is an argument, leaving stdin for the real protocol.
        let mut command = Command::new("sh");
        command.args(["-c", source, "fake-staramp", "embed", "--stdio"]);
        command
    }

    fn presentation(generation: u64) -> Presentation {
        Presentation {
            generation,
            width: 40,
            height: 8,
            focused: true,
            theme: Palette {
                bg: [0, 0, 0],
                fg: [255, 255, 255],
                muted: [128, 128, 128],
                accent: [50, 100, 200],
                selected: [20, 20, 40],
                border: [100, 100, 100],
                error: [255, 0, 0],
            },
            graphics: None,
        }
    }

    #[test]
    fn bad_frames_cannot_reach_the_terminal() {
        let cell = Cell {
            symbol: "\u{1b}[31m".into(),
            fg: [0; 3],
            bg: [0; 3],
            modifiers: 0,
        };
        assert!(validate_frame(
            &Frame {
                generation: 1,
                width: 1,
                height: 1,
                cells: vec![cell],
                images: vec![],
            },
            None
        )
        .is_err());
        assert!(validate_frame(
            &Frame {
                generation: 1,
                width: 2,
                height: 1,
                cells: vec![],
                images: vec![],
            },
            None
        )
        .is_err());
    }

    #[test]
    fn old_protocol_messages_default_optional_graphics_fields() {
        let hello: ChildMessage =
            serde_json::from_str(r#"{"type":"hello","protocol":1,"extensions":["mp3"]}"#).unwrap();
        assert!(
            matches!(hello, ChildMessage::Hello { capabilities, .. } if capabilities.is_empty())
        );
        let old_frame: ChildMessage = serde_json::from_str(
            r#"{"type":"frame","generation":1,"width":1,"height":1,"cells":[{"symbol":"x","fg":[1,2,3],"bg":[4,5,6],"modifiers":0}]}"#,
        ).unwrap();
        let ChildMessage::Frame {
            generation,
            width,
            height,
            cells,
            images,
        } = old_frame
        else {
            panic!("frame expected")
        };
        let frame = Frame {
            generation,
            width,
            height,
            cells,
            images,
        };
        assert!(frame.images.is_empty());
        assert!(validate_frame(&frame, None).is_ok());

        let theme = presentation(1).theme;
        let legacy = serde_json::to_value(HostMessage::Configure {
            generation: 1,
            width: 1,
            height: 1,
            focused: true,
            theme: &theme,
            graphics: None,
            profile: None,
        })
        .unwrap();
        assert!(legacy.get("graphics").is_none());
        assert!(legacy.get("profile").is_none());
        let config = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let extended = serde_json::to_value(HostMessage::Configure {
            generation: 1,
            width: 1,
            height: 1,
            focused: true,
            theme: &theme,
            graphics: Some(&config),
            profile: Some("starfold"),
        })
        .unwrap();
        assert_eq!(extended["graphics"]["cell_width"], 8);
        assert_eq!(extended["profile"], "starfold");
    }

    #[test]
    fn transport_images_require_exact_safe_geometry_and_bytes() {
        let config = GraphicsConfig {
            cell_width: 4,
            cell_height: 3,
        };
        let mut frame = blank_frame(5, 3);
        frame.images.push(image(1, 1, 2, 1, config));
        assert!(validate_frame(&frame, Some(&config)).is_ok());
        assert!(validate_frame(&frame, None).is_err());

        let mut malformed = frame.clone();
        malformed.images[0].x = 4;
        assert!(validate_frame(&malformed, Some(&config)).is_err());
        malformed = frame.clone();
        malformed.images[0].pixel_width -= 1;
        assert!(validate_frame(&malformed, Some(&config)).is_err());
        malformed = frame.clone();
        malformed.images[0].rgba.pop();
        assert!(validate_frame(&malformed, Some(&config)).is_err());
        malformed = frame.clone();
        malformed.images[0].height = 0;
        assert!(validate_frame(&malformed, Some(&config)).is_err());
        malformed = frame;
        malformed.images = vec![image(0, 0, 1, 1, config); MAX_TRANSPORT_IMAGES + 1];
        assert!(validate_frame(&malformed, Some(&config)).is_err());
    }

    #[test]
    fn transport_image_pixel_and_total_caps_are_enforced() {
        let config = GraphicsConfig {
            cell_width: 64,
            cell_height: 128,
        };
        let mut oversized = blank_frame(10, 1);
        oversized.images.push(TransportImage {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
            pixel_width: 640,
            pixel_height: 128,
            rgba: vec![],
        });
        assert!(validate_frame(&oversized, Some(&config)).is_err());

        let config = GraphicsConfig {
            cell_width: 20,
            cell_height: 10,
        };
        let mut total = blank_frame(40, 10);
        total.images.push(image(0, 0, 20, 10, config));
        total.images.push(image(20, 0, 20, 10, config));
        assert!(validate_frame(&total, Some(&config)).is_err());
        assert!(validate_frame(
            &total,
            Some(&GraphicsConfig {
                cell_width: 0,
                cell_height: 10
            })
        )
        .is_err());
    }

    #[test]
    fn child_messages_cannot_carry_terminal_controls() {
        assert_eq!(safe_message("bad\u{1b}[31m\nname\u{009b}"), "bad[31mname");
        let client = Client::with_executable(PathBuf::from("missing-staramp"));
        {
            let mut data = lock(&client.shared);
            data.request_generation = 7;
        }
        push_event(
            &client.shared,
            Event::Error {
                generation: 7,
                message: "\u{1b}oops\n".into(),
            },
        );
        assert_eq!(
            lock(&client.shared).events.front(),
            Some(&Event::Error {
                generation: 7,
                message: "oops".into()
            })
        );
        push_event(
            &client.shared,
            Event::Notice {
                generation: 7,
                message: "save\u{1b} warning".into(),
            },
        );
        assert_eq!(
            lock(&client.shared).events.back(),
            Some(&Event::Notice {
                generation: 7,
                message: "save warning".into()
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn fake_child_handshake_play_and_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper("#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"]}'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *play*) printf '%s\\n' '{\"type\":\"status\",\"playing\":true,\"paused\":false,\"title\":\"Song\"}' ;;\n    *shutdown*) exit 0 ;;\n  esac\ndone\n");
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        // A stopped helper can be launched again with fresh protocol pipes.
        for generation in 1..=2 {
            client
                .activate(song.clone(), vec![song.clone()], presentation(generation))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut accepted = false;
            while Instant::now() < deadline && !accepted {
                accepted = client
                    .take_events()
                    .iter()
                    .any(|e| matches!(e, Event::Accepted { generation: g } if *g == generation));
                thread::sleep(Duration::from_millis(10));
            }
            assert!(accepted);
            assert_eq!(client.supports(Path::new("/tmp/other.mp3")), Some(true));
            assert_eq!(client.supports(Path::new("/tmp/other.txt")), Some(false));
            client.stop().unwrap();
            // stop is asynchronous; let the supervisor observe it before
            // asking this test's helper to start a separate session.
            let deadline = Instant::now() + Duration::from_secs(3);
            while client.stopping.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "stop was not observed");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn helper_capability_is_available_after_handshake() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper(
            "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"],\"capabilities\":[\"transport_images\"]}'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *shutdown*) exit 0 ;;\n  esac\ndone\n",
        );
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        assert!(!client.transport_images_available());
        client
            .activate(song.clone(), vec![song], presentation(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !client.transport_images_available() {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            client.transport_images_available(),
            "handshake events: {:?}",
            client.take_events()
        );
        client.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stale_image_frames_after_button_toggle_and_resize_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper(
            r##"#!/bin/sh
printf '%s\n' '{"type":"hello","protocol":1,"extensions":["mp3"],"capabilities":["transport_images"]}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"configure","generation":2'*)
      printf '%s\n' '{"type":"frame","generation":1,"width":1,"height":1,"cells":[{"symbol":"x","fg":[1,2,3],"bg":[4,5,6],"modifiers":0}],"images":[{"x":0,"y":0,"width":1,"height":1,"pixel_width":1,"pixel_height":1,"rgba":[0,0,0,0]}]}'
      printf '%s\n' '{"type":"status","playing":true,"paused":false,"title":"toggle sent"}'
      ;;
    *'"type":"configure","generation":3'*)
      printf '%s\n' '{"type":"frame","generation":2,"width":1,"height":1,"cells":[{"symbol":"x","fg":[1,2,3],"bg":[4,5,6],"modifiers":0}],"images":[{"x":0,"y":0,"width":1,"height":1,"pixel_width":1,"pixel_height":1,"rgba":[0,0,0,0]}]}'
      printf '%s\n' '{"type":"status","playing":true,"paused":false,"title":"resize sent"}'
      ;;
    *'"type":"shutdown"'*) exit 0 ;;
  esac
done
"##,
        );
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        let mut initial = presentation(1);
        initial.graphics = Some(GraphicsConfig {
            cell_width: 1,
            cell_height: 1,
        });
        client.activate(song.clone(), vec![song], initial).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && !client
                .take_events()
                .iter()
                .any(|event| matches!(event, Event::Accepted { generation: 1 }))
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(client.transport_images_available());

        let mut toggled = presentation(2);
        toggled.graphics = None;
        client.configure(toggled).unwrap();
        let mut observed = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && !observed.iter().any(|event| {
                matches!(event, Event::Status { status, .. } if status.title == "toggle sent")
            })
        {
            observed.extend(client.take_events());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(observed.iter().any(|event| {
            matches!(event, Event::Status { status, .. } if status.title == "toggle sent")
        }));
        thread::sleep(Duration::from_millis(50));
        observed.extend(client.take_events());
        assert!(!observed
            .iter()
            .any(|event| matches!(event, Event::Error { .. })));
        assert!(client.take_frame().is_none());

        let mut resized = presentation(3);
        resized.graphics = Some(GraphicsConfig {
            cell_width: 2,
            cell_height: 2,
        });
        client.configure(resized).unwrap();
        observed.clear();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && !observed.iter().any(|event| {
                matches!(event, Event::Status { status, .. } if status.title == "resize sent")
            })
        {
            observed.extend(client.take_events());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(observed.iter().any(|event| {
            matches!(event, Event::Status { status, .. } if status.title == "resize sent")
        }));
        thread::sleep(Duration::from_millis(50));
        observed.extend(client.take_events());
        assert!(!observed
            .iter()
            .any(|event| matches!(event, Event::Error { .. })));
        assert!(client.take_frame().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn rapid_activation_and_resize_keep_only_the_new_request() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper("#!/bin/sh\nsleep 0.08\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"]}'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *play*) sleep 0.08; printf '%s\\n' '{\"type\":\"frame\",\"generation\":3,\"width\":1,\"height\":1,\"cells\":[{\"symbol\":\"x\",\"fg\":[1,2,3],\"bg\":[4,5,6],\"modifiers\":0}]}' ;;\n    *shutdown*) exit 0 ;;\n  esac\ndone\n");
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        client
            .activate(song.clone(), vec![song.clone()], presentation(1))
            .unwrap();
        client
            .activate(song.clone(), vec![song], presentation(2))
            .unwrap();
        client.configure(presentation(3)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut accepted = false;
        let mut frame = None;
        while Instant::now() < deadline && (!accepted || frame.is_none()) {
            let events = client.take_events();
            assert!(events.iter().all(|event| event.generation() == 2));
            accepted |= events
                .iter()
                .any(|event| matches!(event, Event::Accepted { generation: 2 }));
            frame = frame.or_else(|| client.take_frame());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(accepted);
        assert_eq!(frame.unwrap().generation, 3);
    }

    #[cfg(unix)]
    #[test]
    fn stop_invalidates_pending_start_and_fallback() {
        let mut client = Client::with_executable(PathBuf::from("/definitely/not/staramp"));
        client
            .activate(PathBuf::from("/tmp/song.mp3"), vec![], presentation(1))
            .unwrap();
        client.stop().unwrap();
        thread::sleep(Duration::from_millis(80));
        assert!(client.take_events().is_empty());
        assert!(client.take_frame().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_playlist_path_is_reported_before_acceptance() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper("#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"]}'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *shutdown*) exit 0 ;;\n  esac\ndone\n");
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let invalid = dir.path().join(std::ffi::OsString::from_vec(vec![
            b'x', 0xff, b'.', b'm', b'p', b'3',
        ]));
        let mut client = Client::with_command(script);
        client
            .activate(song.clone(), vec![song, invalid], presentation(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut events = Vec::new();
        while Instant::now() < deadline && events.is_empty() {
            events.extend(client.take_events());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Error { message, .. } if message.contains("non-UTF-8"))));
        assert!(!events.iter().any(|e| matches!(e, Event::Accepted { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn drop_kills_and_reaps_a_child_that_ignores_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper("#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"]}'\nwhile :; do sleep 1; done\n");
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        client
            .activate(song.clone(), vec![song], presentation(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && client.supports(Path::new("/tmp/song.mp3")) != Some(true)
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            client.supports(Path::new("/tmp/song.mp3")),
            Some(true),
            "handshake events: {:?}",
            client.take_events()
        );
        let start = Instant::now();
        drop(client);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn missing_old_and_silent_helpers_fall_back_before_playback() {
        let selected = PathBuf::from("/tmp/song.mp3");
        let mut missing = Client::with_executable(PathBuf::from("/definitely/not/staramp"));
        missing
            .activate(selected.clone(), vec![], presentation(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut missing_event = None;
        while Instant::now() < deadline && missing_event.is_none() {
            missing_event = missing.take_events().into_iter().next();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            missing_event,
            Some(Event::Fallback { generation: 1, .. })
        ));

        let old = fake_helper("#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":2,\"extensions\":[\"mp3\"]}'\nsleep 5\n");
        let mut old_client = Client::with_command(old);
        old_client
            .activate(selected.clone(), vec![], presentation(2))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut old_event = None;
        while Instant::now() < deadline && old_event.is_none() {
            old_event = old_client.take_events().into_iter().next();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            old_event,
            Some(Event::Fallback { generation: 2, .. })
        ));

        let silent = fake_helper("#!/bin/sh\nsleep 5\n");
        let mut silent_client = Client::with_command(silent);
        silent_client
            .activate(selected, vec![], presentation(3))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut silent_event = None;
        while Instant::now() < deadline && silent_event.is_none() {
            silent_event = silent_client.take_events().into_iter().next();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            silent_event,
            Some(Event::Fallback { generation: 3, .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn helper_exit_after_acceptance_is_an_error_without_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_helper("#!/bin/sh\nprintf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"extensions\":[\"mp3\"]}'\nwhile IFS= read -r line; do\n  case \"$line\" in\n    *play*) printf '%s\\n' '{\"type\":\"status\",\"playing\":true,\"paused\":false,\"title\":\"Song\"}'; sleep 0.1; exit 0 ;;\n  esac\ndone\n");
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        let mut client = Client::with_command(script);
        client
            .activate(song.clone(), vec![song], presentation(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut events = Vec::new();
        while Instant::now() < deadline && !events.iter().any(|e| matches!(e, Event::Error { .. }))
        {
            events.extend(client.take_events());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Accepted { generation: 1 })));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Error { generation: 1, .. })));
        assert!(!events.iter().any(|e| matches!(e, Event::Fallback { .. })));
    }
}
