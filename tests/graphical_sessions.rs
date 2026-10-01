//! Exercise the real detached controller, with no display server or renderer.
#![cfg(all(unix, feature = "terminal-graphics"))]
use starkit::terminal_graphics::protocol::*;
use std::{
    fs,
    io::{BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Host {
    child: Child,
    _directory: tempfile::TempDir,
    socket: PathBuf,
    files: PathBuf,
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Host {
    fn start() -> Self {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let files = directory.path().join("files");
        fs::create_dir(&files).unwrap();
        fs::create_dir(files.join("destination")).unwrap();
        let mut file = fs::File::create(files.join("one 日本語.bin")).unwrap();
        for _ in 0..64 {
            file.write_all(&vec![7u8; 1024 * 1024]).unwrap();
        }
        fs::write(files.join("two ' quoted.bin"), b"second marked file").unwrap();
        let base = directory.path().join("app");
        let child = Command::new(env!("CARGO_BIN_EXE_starfold"))
            .args(["--graphical-server", "test", "--directory"])
            .arg(&files)
            .env("STARFOLD_DIR", &base)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let socket = base.join("graphical/test.sock");
        let started = Instant::now();
        while !socket.exists() {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        Self {
            child,
            _directory: directory,
            socket,
            files,
        }
    }
    fn connect(&self) -> (UnixStream, BufReader<UnixStream>) {
        let mut socket = UnixStream::connect(&self.socket).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write_message(
            &ClientMessage::Hello {
                version: VERSION,
                viewport: Viewport::default(),
                client: "persistent-test".into(),
            },
            &mut socket,
        )
        .unwrap();
        let reader = BufReader::new(socket.try_clone().unwrap());
        (socket, reader)
    }
}
fn wait(
    reader: &mut BufReader<UnixStream>,
    test: impl Fn(&ServerMessage) -> bool,
) -> ServerMessage {
    let start = Instant::now();
    loop {
        let m = read_message::<ServerMessage>(reader)
            .unwrap()
            .expect("session unexpectedly closed");
        assert!(start.elapsed() < Duration::from_secs(10));
        if test(&m) {
            return m;
        }
    }
}
fn key(socket: &mut UnixStream, reader: &mut BufReader<UnixStream>, id: u64, code: &str) -> bool {
    write_message(
        &ClientMessage::Input {
            id,
            revision: 0,
            generation: 1,
            input: Input::Key {
                code: code.into(),
                modifiers: 0,
            },
        },
        socket,
    )
    .unwrap();
    matches!(
        wait(
            reader,
            |m| matches!(m,ServerMessage::Ack{id:received,..} if *received==id)
        ),
        ServerMessage::Ack { accepted: true, .. }
    )
}
#[test]
fn marked_copy_survives_detach_and_duplicate_input_is_rejected() {
    let host = Host::start();
    let (mut socket, mut reader) = host.connect();
    let hello = wait(&mut reader, |m| matches!(m, ServerMessage::Hello { .. }));
    wait(
        &mut reader,
        |m| matches!(m,ServerMessage::Scene{scene} if scene.components.iter().any(|c|matches!(c,Component::ListRow{label,..} if label.starts_with("one")))),
    );
    // Mark both files, capture the marked set, enter the destination and paste.
    for (index, code) in [
        "down", "char: ", "down", "char: ", "char:y", "home", "enter", "char:p",
    ]
    .iter()
    .enumerate()
    {
        assert!(key(&mut socket, &mut reader, index as u64 + 1, code));
    }
    socket.shutdown(std::net::Shutdown::Both).unwrap();
    drop(reader);
    drop(socket);
    let start = Instant::now();
    while !host.files.join("destination/two ' quoted.bin").exists() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read(host.files.join("destination/two ' quoted.bin")).unwrap(),
        b"second marked file"
    );
    assert_eq!(
        fs::metadata(host.files.join("destination/one 日本語.bin"))
            .unwrap()
            .len(),
        64 * 1024 * 1024
    );
    let (mut socket, mut reader) = host.connect();
    let again = wait(&mut reader, |m| matches!(m, ServerMessage::Hello { .. }));
    match (hello, again) {
        (ServerMessage::Hello { epoch: a, .. }, ServerMessage::Hello { epoch: b, .. }) => {
            assert_eq!(a, b)
        }
        _ => unreachable!(),
    }
    assert!(!key(&mut socket, &mut reader, 8, "char:p"));
    assert!(key(&mut socket, &mut reader, 9, "char:q"));
}

#[test]
fn headless_host_retains_places_and_operations_actions() {
    let host = Host::start();
    let (mut socket, mut reader) = host.connect();
    wait(&mut reader, |m| matches!(m, ServerMessage::Hello { .. }));
    assert!(key(&mut socket, &mut reader, 1, "char:b"));
    wait(
        &mut reader,
        |m| matches!(m,ServerMessage::Scene{scene} if scene.spans.iter().any(|s|s.text.contains("PLACES"))),
    );
    assert!(key(&mut socket, &mut reader, 2, "escape"));
    write_message(
        &ClientMessage::Input {
            id: 3,
            revision: 0,
            generation: 1,
            input: Input::Key {
                code: "char:3".into(),
                modifiers: 4,
            },
        },
        &mut socket,
    )
    .unwrap();
    wait(&mut reader, |m| {
        matches!(m, ServerMessage::Ack { id: 3, .. })
    });
    let scene = wait(
        &mut reader,
        |m| matches!(m,ServerMessage::Scene{scene} if scene.spans.iter().any(|s|s.text.contains("OPERATIONS"))),
    );
    assert!(matches!(scene, ServerMessage::Scene { .. }));
}

#[test]
fn socket_probe_does_not_steal_an_active_attachment() {
    let host = Host::start();
    let (mut socket, mut reader) = host.connect();
    wait(&mut reader, |m| matches!(m, ServerMessage::Hello { .. }));
    drop(UnixStream::connect(&host.socket).unwrap());
    std::thread::sleep(Duration::from_millis(100));
    write_message(&ClientMessage::Ping, &mut socket).unwrap();
    wait(&mut reader, |m| matches!(m, ServerMessage::Pong));
    assert!(key(&mut socket, &mut reader, 1, "char:q"));
}

#[test]
fn remote_desktop_drop_routes_acknowledged_effects_to_the_controller() {
    use base64::Engine as _;
    let host = Host::start();
    let (mut socket, mut reader) = host.connect();
    wait(&mut reader, |m| matches!(m, ServerMessage::Hello { .. }));
    let scene = wait(&mut reader, |m| {
        matches!(m, ServerMessage::Scene { scene }
        if scene.components.iter().any(|c| matches!(c, Component::ListRow { label, .. } if label == "destination/")))
    });
    let (x, y) = match scene {
        ServerMessage::Scene { scene } => scene
            .components
            .iter()
            .find_map(|c| match c {
                Component::ListRow { rect, label, .. } if label == "destination/" => {
                    Some((rect.x + 4, rect.y))
                }
                _ => None,
            })
            .unwrap(),
        _ => unreachable!(),
    };
    let mut id = 0;
    let mut send = |text: String, socket: &mut UnixStream| {
        id += 1;
        write_message(
            &ClientMessage::Input {
                id,
                revision: 0,
                generation: 1,
                input: Input::Osc72 { text },
            },
            socket,
        )
        .unwrap();
    };
    fn effect(socket: &mut UnixStream, reader: &mut BufReader<UnixStream>, prefix: &str) {
        loop {
            let message = read_message::<ServerMessage>(reader).unwrap().unwrap();
            if let ServerMessage::Osc72 { id, meta, .. } = message {
                write_message(&ClientMessage::EffectAck { id }, socket).unwrap();
                if meta.starts_with(prefix) {
                    return;
                }
            }
        }
    }
    send("t=q:i=1".into(), &mut socket);
    effect(&mut socket, &mut reader, "t=a:i=1");
    send(
        format!("t=m:x={x}:y={y}:o=1:i=1;text/uri-list"),
        &mut socket,
    );
    effect(&mut socket, &mut reader, "t=m:o=1");
    send(
        format!("t=M:x={x}:y={y}:o=1:i=1;text/uri-list"),
        &mut socket,
    );
    effect(&mut socket, &mut reader, "t=r:x=1:i=1");
    let uri = base64::engine::general_purpose::STANDARD_NO_PAD
        .encode("file:///Users/test/from%20Mac.txt\r\n");
    send(format!("t=r:x=1:m=0:X=1:i=1;{uri}"), &mut socket);
    send("m=0:i=1".into(), &mut socket);
    effect(&mut socket, &mut reader, "t=r:x=1:y=1:i=1");
    let data = base64::engine::general_purpose::STANDARD_NO_PAD.encode(b"graphical remote drop");
    send(format!("t=r:x=1:y=1:m=0:i=1;{data}"), &mut socket);
    send("m=0:i=1".into(), &mut socket);
    let started = Instant::now();
    let target = host.files.join("destination/from Mac.txt");
    while !target.exists() {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read(target).unwrap(), b"graphical remote drop");
}
