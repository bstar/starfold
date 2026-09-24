//! Kitty's OSC 72 drag-and-drop wire format. Only the terminal layer touches
//! these messages; filesystem changes still go through the fold core.
//!
//! https://sw.kovidgoyal.net/kitty/dnd-protocol/

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::fold::ops::{progress::Progress, OpId, OpKind};

use base64::Engine as _;
use percent_encoding::{percent_decode, percent_encode, AsciiSet, CONTROLS};
use sha2::{Digest, Sha256};

/// A whole OSC 72 escape without the `ESC ] 72 ;` and `ESC \\` framing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message<'a> {
    pub metadata: Vec<(&'a str, &'a str)>,
    pub payload: &'a str,
}

impl<'a> Message<'a> {
    pub fn parse(raw: &'a str) -> Option<Self> {
        let (meta, payload) = raw.split_once(';').unwrap_or((raw, ""));
        let mut metadata = Vec::new();
        for part in meta.split(':') {
            let (key, value) = part.split_once('=')?;
            if key.is_empty() || value.contains(['\x1b', '\x07']) {
                return None;
            }
            metadata.push((key, value));
        }
        if metadata.is_empty() {
            return None;
        }
        Some(Self { metadata, payload })
    }

    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.metadata
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }

    pub fn number(&self, key: &str) -> Option<i32> {
        self.get(key)?.parse().ok()
    }

    pub fn data(&self) -> Option<Vec<u8>> {
        base64::engine::general_purpose::STANDARD
            .decode(self.payload)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(self.payload))
            .ok()
    }

    /// Kitty can mark a nonempty chunk `m=0` and still send a separate empty
    /// message to close the stream. Only the empty message ends the data.
    pub fn end_of_data(&self) -> bool {
        self.payload.is_empty() && self.number("m").unwrap_or(0) == 0
    }
}

/// Write one complete escape. Callers own stdout serialization with rendering.
pub fn send(meta: &str, payload: Option<&str>) -> io::Result<()> {
    let mut out = io::stdout().lock();
    write!(out, "\x1b]72;{meta}")?;
    if let Some(payload) = payload {
        write!(out, ";{payload}")?;
    }
    out.write_all(b"\x1b\\")?;
    out.flush()
}

pub fn send_data(meta: &str, bytes: &[u8]) -> io::Result<()> {
    // The last data chunk still has m=1; the empty m=0 message ends the
    // stream. Kitty's streaming decoder rejects '=' padding before that end.
    const RAW_CHUNK: usize = 3072; // 4096 base64 bytes, the protocol limit.
    for part in bytes.chunks(RAW_CHUNK) {
        send(
            &format!("{meta}:m=1"),
            Some(&base64::engine::general_purpose::STANDARD_NO_PAD.encode(part)),
        )?;
    }
    send(&format!("{meta}:m=0"), Some(""))
}

// The URI list is ASCII on the wire. Delimiters and '%' must be escaped;
// bytes outside ASCII preserve Unix filenames that are not valid UTF-8.
const URI_PATH: &AsciiSet = &CONTROLS
    .add(b'%')
    .add(b' ')
    .add(b'#')
    .add(b'?')
    .add(b'\r')
    .add(b'\n');

#[cfg(unix)]
pub fn file_uri(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    path.is_absolute()
        .then(|| format!("file://{}", percent_encode(bytes, URI_PATH)))
}

#[cfg(unix)]
pub fn parse_file_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let authority_and_path = uri.strip_prefix("file://")?;
    let path = if authority_and_path.starts_with('/') {
        authority_and_path
    } else {
        &authority_and_path[authority_and_path.find('/')?..]
    };
    let bytes = percent_decode(path.as_bytes()).collect::<Vec<_>>();
    if bytes.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

pub fn uri_list(payload: &[u8]) -> Option<Vec<PathBuf>> {
    let text = std::str::from_utf8(payload).ok()?;
    let paths: Vec<_> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(parse_file_uri)
        .collect::<Option<_>>()?;
    (!paths.is_empty()).then_some(paths)
}

/// Machine IDs are HMAC-SHA256, per OSC 72, so the OS machine ID is not
/// exposed to whatever program can read the terminal stream.
pub fn machine_id() -> Option<String> {
    let raw = if cfg!(target_os = "macos") {
        let output = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .ok()?;
        let listing = String::from_utf8(output.stdout).ok()?;
        listing.lines().find_map(|line| {
            line.contains("IOPlatformUUID")
                .then(|| line.rsplit('"').nth(1).map(str::to_owned))
                .flatten()
        })?
    } else {
        std::fs::read_to_string("/etc/machine-id")
            .ok()?
            .trim()
            .to_owned()
    };
    let key = b"tty-dnd-protocol-machine-id";
    let mut block = [0u8; 64];
    block[..key.len()].copy_from_slice(key);
    let mut inner_pad = block;
    let mut outer_pad = block;
    for b in &mut inner_pad {
        *b ^= 0x36;
    }
    for b in &mut outer_pad {
        *b ^= 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(raw.as_bytes());
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    let digest = outer.finalize();
    Some(format!(
        "1:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

#[derive(Debug, Clone)]
pub struct Offer {
    pub sources: Vec<PathBuf>,
    pub uri_text: String,
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub dest: PathBuf,
    pub own_sources: Option<Vec<PathBuf>>,
    pub allowed: i32,
    pub mime_index: Option<i32>,
    pub remote: bool,
}

#[derive(Debug)]
pub struct Active {
    pub op: OpId,
    pub result_operation: OpKind,
}

#[derive(Debug, Default)]
pub struct State {
    pub enabled: bool,
    pub offered_uri: bool,
    pub hover: Option<PathBuf>,
    pub hover_coords: Option<(u16, u16)>,
    pub offer: Option<Offer>,
    pub choice: Option<Choice>,
    pub active: Option<Active>,
    pub receiving_uri: bool,
    pub received: Vec<u8>,
    pub result_kind: Option<OpKind>,
    pub remote: Option<Remote>,
    pub staged: Option<tempfile::TempDir>,
    pub import_op: Option<OpId>,
    pub export_op: Option<OpId>,
    pub export_tx: Option<crossbeam_channel::Sender<usize>>,
    pub export_done: Option<crossbeam_channel::Receiver<io::Result<()>>>,
    pub export_output: Option<crossbeam_channel::Receiver<Outgoing>>,
    pub export_drag_finished: bool,
    pub export_cancelled: bool,
}

#[derive(Debug)]
pub struct Outgoing {
    pub meta: String,
    pub payload: Option<String>,
}

fn emit(
    tx: &crossbeam_channel::Sender<Outgoing>,
    meta: String,
    payload: Option<String>,
) -> io::Result<()> {
    tx.send(Outgoing { meta, payload })
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "drag receiver closed"))
}

fn emit_data(tx: &crossbeam_channel::Sender<Outgoing>, meta: &str, bytes: &[u8]) -> io::Result<()> {
    for chunk in bytes.chunks(3072) {
        emit(
            tx,
            format!("{meta}:m=1"),
            Some(base64::engine::general_purpose::STANDARD_NO_PAD.encode(chunk)),
        )?;
    }
    emit(tx, format!("{meta}:m=0"), Some(String::new()))
}

pub fn stream_offer(
    sources: Vec<PathBuf>,
    requests: crossbeam_channel::Receiver<usize>,
    progress: Arc<Progress>,
    output: crossbeam_channel::Sender<Outgoing>,
) -> io::Result<()> {
    let mut total = 0u64;
    for source in &sources {
        total = total.saturating_add(tree_bytes(source)?);
    }
    progress.set_total(total);
    // Handles identify directories for the entire drag, including requests
    // for different top-level URIs that the terminal queued together.
    let mut next_handle = 2i32;
    while let Ok(index) = requests.recv() {
        if index == 0 || index > sources.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid drag item",
            ));
        }
        let mut entries = VecDeque::from([(sources[index - 1].clone(), index, None)]);
        while let Some((path, root, parent)) = entries.pop_front() {
            if progress.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "drag cancelled"));
            }
            let mut meta = format!("t=k:x={root}:i=1");
            if let Some((handle, number)) = parent {
                meta.push_str(&format!(":Y={handle}:y={number}"));
            }
            let info = fs::symlink_metadata(&path)?;
            if info.is_file() {
                let mut file = File::open(&path)?;
                let mut buf = [0u8; 3072];
                loop {
                    let n = std::io::Read::read(&mut file, &mut buf)?;
                    if n == 0 {
                        break;
                    }
                    if progress.is_cancelled() {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "drag cancelled"));
                    }
                    emit(
                        &output,
                        format!("{meta}:m=1"),
                        Some(base64::engine::general_purpose::STANDARD_NO_PAD.encode(&buf[..n])),
                    )?;
                    progress.add(n as u64);
                }
                emit(&output, format!("{meta}:m=0"), Some(String::new()))?;
            } else if info.file_type().is_symlink() {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStrExt;
                    let target = fs::read_link(&path)?;
                    emit_data(
                        &output,
                        &format!("{meta}:X=1"),
                        target.as_os_str().as_bytes(),
                    )?;
                }
            } else if info.is_dir() {
                let handle = next_handle;
                next_handle = next_handle.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::OutOfMemory, "too many drag directories")
                })?;
                let mut children = Vec::new();
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    let ty = entry.file_type()?;
                    if ty.is_file() || ty.is_dir() || ty.is_symlink() {
                        children.push(entry);
                    }
                }
                children.sort_by_key(|entry| entry.file_name());
                let mut names = Vec::new();
                for (number, child) in children.into_iter().enumerate() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::ffi::OsStrExt;
                        names.extend_from_slice(child.file_name().as_bytes());
                    }
                    names.push(0);
                    entries.push_back((child.path(), root, Some((handle, number + 1))));
                }
                emit_data(&output, &format!("{meta}:X={handle}"), &names)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported file type",
                ));
            }
        }
    }
    Ok(())
}

fn tree_bytes(path: &Path) -> io::Result<u64> {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        let info = fs::symlink_metadata(&path)?;
        if info.is_file() {
            total = total.saturating_add(info.len());
        } else if info.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(total)
}

#[derive(Debug)]
struct Task {
    path: PathBuf,
    root_index: usize,
    parent: Option<(i32, usize)>,
}

/// A remote drop is requested one item at a time. This keeps the protocol
/// responsive without buffering file contents or depending on response order.
#[derive(Debug)]
pub struct Remote {
    pub stage: tempfile::TempDir,
    pub roots: Vec<PathBuf>,
    mime_index: i32,
    waiting: VecDeque<Task>,
    current: Option<Task>,
    current_kind: Option<i32>,
    current_file: Option<File>,
    small_data: Vec<u8>,
    remaining: HashMap<i32, usize>,
    progress: Arc<Progress>,
    sender: fn(&str, Option<&str>) -> io::Result<()>,
}

impl Remote {
    pub fn new(
        dest: &Path,
        paths: &[PathBuf],
        mime_index: i32,
        progress: Arc<Progress>,
    ) -> io::Result<Self> {
        let stage = tempfile::Builder::new()
            .prefix(".starfold-drop-")
            .tempdir_in(dest)?;
        let mut roots = Vec::new();
        let mut waiting = VecDeque::new();
        for (i, source) in paths.iter().enumerate() {
            let name = source.file_name().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "drop has no file name")
            })?;
            if !safe_name(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsafe drop file name",
                ));
            }
            let path = stage.path().join(name);
            if roots.contains(&path) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "duplicate drop file name",
                ));
            }
            roots.push(path.clone());
            waiting.push_back(Task {
                path,
                root_index: i + 1,
                parent: None,
            });
        }
        Ok(Self {
            stage,
            roots,
            mime_index,
            waiting,
            current: None,
            current_kind: None,
            current_file: None,
            small_data: Vec::new(),
            remaining: HashMap::new(),
            progress,
            sender: send,
        })
    }

    /// Send the next request, or report that the whole tree was received.
    pub fn request_next(&mut self) -> io::Result<bool> {
        if let Some(task) = self.waiting.pop_front() {
            let meta = match task.parent {
                Some((handle, index)) => format!("t=r:Y={handle}:x={index}:i=1"),
                None => format!("t=r:x={}:y={}:i=1", self.mime_index, task.root_index),
            };
            (self.sender)(&meta, None)?;
            self.current = Some(task);
            self.current_kind = None;
            self.current_file = None;
            self.small_data.clear();
            Ok(false)
        } else {
            Ok(true)
        }
    }

    pub fn receive(&mut self, message: &Message<'_>) -> io::Result<bool> {
        if self.progress.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "drop cancelled"));
        }
        let task = self
            .current
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unsolicited drag data"))?;
        if self.current_kind.is_none() {
            let matching = match task.parent {
                Some((handle, index)) => {
                    message.number("Y") == Some(handle) && message.number("x") == Some(index as i32)
                }
                None => {
                    message.number("x") == Some(self.mime_index)
                        && message.number("y") == Some(task.root_index as i32)
                }
            };
            if !matching {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "remote response does not match request",
                ));
            }
        }
        if let Some(old) = self.current_kind {
            if message.number("X").is_some_and(|kind| kind != old) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "remote entry type changed",
                ));
            }
        } else {
            let kind = message.number("X").unwrap_or(0);
            self.current_kind = Some(kind);
            if kind == 0 && self.current_file.is_none() {
                self.current_file = Some(File::create(&task.path)?);
            } else if kind >= 2 {
                fs::create_dir(&task.path)?;
            } else if kind != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid remote entry type",
                ));
            }
        }
        let kind = self.current_kind.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "remote entry has no type")
        })?;
        let data = message
            .data()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid remote base64"))?;
        if kind == 0 {
            self.current_file.as_mut().unwrap().write_all(&data)?;
            self.progress.add(data.len() as u64);
        } else {
            if self.small_data.len().saturating_add(data.len()) > 8 * 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "remote entry metadata too large",
                ));
            }
            self.small_data.extend(data);
        }
        if !message.end_of_data() {
            return Ok(false);
        }
        self.current_file = None;
        let task = self.current.take().unwrap();
        if kind == 1 {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                std::os::unix::fs::symlink(
                    OsString::from_vec(std::mem::take(&mut self.small_data)),
                    &task.path,
                )?;
            }
        } else if kind >= 2 {
            let names = split_names(&self.small_data)?;
            let count = names.len();
            if self.remaining.insert(kind, count).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "reused remote directory handle",
                ));
            }
            for (i, name) in names.into_iter().enumerate() {
                self.waiting.push_back(Task {
                    path: task.path.join(name),
                    root_index: task.root_index,
                    parent: Some((kind, i + 1)),
                });
            }
            if count == 0 {
                (self.sender)(&format!("t=r:Y={kind}:i=1"), None)?;
                self.remaining.remove(&kind);
            }
        }
        if let Some((handle, _)) = task.parent {
            let remaining = self.remaining.get_mut(&handle).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown remote directory handle",
                )
            })?;
            *remaining -= 1;
            if *remaining == 0 {
                (self.sender)(&format!("t=r:Y={handle}:i=1"), None)?;
                self.remaining.remove(&handle);
            }
        }
        self.request_next()
    }
}

#[cfg(unix)]
fn safe_name(name: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let b = name.as_bytes();
    !b.is_empty() && b != b"." && b != b".." && !b.contains(&b'/') && !b.contains(&0)
}

#[cfg(unix)]
fn split_names(bytes: &[u8]) -> io::Result<Vec<OsString>> {
    use std::os::unix::ffi::OsStringExt;
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in bytes.split(|b| *b == 0).filter(|name| !name.is_empty()) {
        let os = OsString::from_vec(name.to_vec());
        if !safe_name(&os) || !seen.insert(os.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsafe or duplicate remote file name",
            ));
        }
        result.push(os);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn osc_fields_and_payload() {
        let m = Message::parse("t=M:x=3:y=4:o=3;text/uri-list").unwrap();
        assert_eq!(m.get("t"), Some("M"));
        assert_eq!(m.number("x"), Some(3));
        assert_eq!(m.payload, "text/uri-list");
        assert!(Message::parse("broken").is_none());
        assert!(!Message::parse("t=r:m=0;YQ").unwrap().end_of_data());
        assert!(Message::parse("m=0;").unwrap().end_of_data());
    }

    #[test]
    fn uri_round_trip_keeps_special_and_non_utf8_names() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(b"/tmp/a #\xff.txt".to_vec()));
        let uri = file_uri(&path).unwrap();
        assert_eq!(parse_file_uri(&uri), Some(path));
    }

    proptest! {
        #[test]
        fn uri_round_trip_for_arbitrary_unix_names(name in proptest::collection::vec(1u8..=255, 1..64)) {
            use std::os::unix::ffi::OsStringExt;
            let name: Vec<u8> = name.into_iter().filter(|b| *b != b'/').collect();
            prop_assume!(!name.is_empty());
            let path = PathBuf::from("/tmp").join(OsString::from_vec(name));
            let uri = file_uri(&path).unwrap();
            prop_assert_eq!(parse_file_uri(&uri), Some(path));
        }
    }

    #[test]
    fn accepts_unpadded_data_and_rejects_duplicate_remote_names() {
        assert_eq!(
            Message::parse("t=r;YQ").unwrap().data(),
            Some(b"a".to_vec())
        );
        assert!(split_names(b"safe\0safe\0").is_err());
        assert!(split_names(b"..\0").is_err());
    }

    #[test]
    fn exported_directory_is_breadth_first_and_ends_each_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("folder");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("first"), b"abc").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested").join("second"), b"def").unwrap();
        let (requests_tx, requests_rx) = crossbeam_channel::bounded(1);
        let (output_tx, output_rx) = crossbeam_channel::bounded(32);
        requests_tx.send(1).unwrap();
        drop(requests_tx);
        let progress = Arc::new(Progress::new(0));
        stream_offer(vec![root], requests_rx, Arc::clone(&progress), output_tx).unwrap();
        let messages: Vec<_> = output_rx.try_iter().collect();
        assert!(messages.first().unwrap().meta.contains("X=2"));
        assert!(messages.iter().any(|m| m.meta.contains("Y=2:y=1:m=1")));
        assert!(messages.iter().any(|m| m.meta.contains("Y=2:y=2:X=3:m=1")));
        assert!(messages.iter().any(|m| m.meta.contains("Y=3:y=1:m=1")));
        assert_eq!(progress.done(), 6);
    }

    #[test]
    fn separate_remote_requests_get_distinct_directory_handles() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();
        let (requests_tx, requests_rx) = crossbeam_channel::bounded(2);
        let (output_tx, output_rx) = crossbeam_channel::bounded(4);
        requests_tx.send(1).unwrap();
        requests_tx.send(2).unwrap();
        drop(requests_tx);
        stream_offer(
            vec![left, right],
            requests_rx,
            Arc::new(Progress::new(0)),
            output_tx,
        )
        .unwrap();
        let messages: Vec<_> = output_rx.try_iter().collect();
        assert!(messages[0].meta.contains("x=1") && messages[0].meta.contains("X=2"));
        assert!(messages[1].meta.contains("x=2") && messages[1].meta.contains("X=3"));
    }

    #[test]
    fn remote_directory_chunks_stage_a_tree_without_escaping_destination() {
        let dest = tempfile::tempdir().unwrap();
        let progress = Arc::new(Progress::new(0));
        let mut remote = Remote::new(
            dest.path(),
            &[PathBuf::from("/elsewhere/folder")],
            1,
            Arc::clone(&progress),
        )
        .unwrap();
        remote.sender = |_, _| Ok(());
        assert!(!remote.request_next().unwrap());
        let names = base64::engine::general_purpose::STANDARD.encode(b"child\0");
        let first_wire = format!("t=r:x=1:y=1:X=2:m=0;{names}");
        let first = Message::parse(&first_wire).unwrap();
        assert!(!remote.receive(&first).unwrap());
        assert!(!remote.receive(&Message::parse("m=0;").unwrap()).unwrap());
        let data = base64::engine::general_purpose::STANDARD.encode(b"content");
        let child_wire = format!("t=r:Y=2:x=1:m=0;{data}");
        let child = Message::parse(&child_wire).unwrap();
        assert!(!remote.receive(&child).unwrap());
        assert!(remote.receive(&Message::parse("m=0;").unwrap()).unwrap());
        assert_eq!(fs::read(remote.roots[0].join("child")).unwrap(), b"content");
        assert_eq!(progress.done(), 7);
        assert!(remote.roots[0].starts_with(dest.path()));
    }
}
