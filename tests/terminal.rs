//! A real terminal session whose far end never answers terminal queries.
#![cfg(unix)]

use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Running {
    child: Child,
    master: Option<File>,
}
impl Drop for Running {
    fn drop(&mut self) {
        // BSD terminal teardown can wait for unread output even after SIGKILL.
        // Closing the controller first also makes failure cleanup bounded.
        drop(self.master.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty initializes two descriptors and only borrows size here.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(size),
            )
        },
        0
    );
    // SAFETY: both descriptors are newly owned, and transferred exactly once.
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for file in [&master, &slave] {
        // SAFETY: these calls operate on live descriptors without pointers.
        assert_eq!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
    }
    // A nonblocking reader lets test failures terminate/reap the child promptly.
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
        0
    );
    (master, slave)
}

fn frame(child: &mut Running) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut last_output = Instant::now();
    loop {
        let mut bytes = [0; 16384];
        match child.master.as_mut().unwrap().read(&mut bytes) {
            Ok(n) => {
                output.extend_from_slice(&bytes[..n]);
                last_output = Instant::now();
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if output.windows(7).any(|w| w == b"PREVIEW")
                    && last_output.elapsed() >= Duration::from_millis(20)
                {
                    return output;
                }
            }
            Err(e) => panic!(
                "PTY read: {e}; output: {}",
                String::from_utf8_lossy(&output)
            ),
        }
        assert!(
            !output.windows(4).any(|w| w == b"\x1b[6n"),
            "Repaint queried the cursor position"
        );
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "Exited before drawing: {}",
            String::from_utf8_lossy(&output)
        );
        assert!(
            Instant::now() < deadline,
            "No frame: {}",
            String::from_utf8_lossy(&output)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn collect_for(child: &mut Running, duration: Duration) -> Vec<u8> {
    let deadline = Instant::now() + duration;
    let mut output = Vec::new();
    while Instant::now() < deadline {
        let mut bytes = [0u8; 16384];
        match child.master.as_mut().unwrap().read(&mut bytes) {
            Ok(n) => output.extend_from_slice(&bytes[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("PTY read: {e}"),
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    output
}

#[test]
fn startup_redraw_and_resize_need_no_cursor_position_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let files = tmp.path().join("files");
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&files).unwrap();
    let (master, slave) = pty();
    let mut command = Command::new(env!("CARGO_BIN_EXE_starfold"));
    command
        .arg(&files)
        .env("STARFOLD_DIR", &config)
        .env("STARFOLD_CONFIG_DIR", &config)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    // Keep the capability probe out of this test: the regression is in
    // fullscreen repaint, which must work with no terminal answers at all.
    for key in [
        "TMUX",
        "TERM_PROGRAM",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "SSH_TTY",
        "SSH_CONNECTION",
    ] {
        command.env_remove(key);
    }
    // SAFETY: the post-fork closure only calls async-signal-safe libc APIs.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Running {
        child: command.spawn().unwrap(),
        master: Some(master),
    };
    frame(&mut child);
    child.master.as_mut().unwrap().write_all(b"\x0c").unwrap();
    frame(&mut child);
    let size = libc::winsize {
        ws_row: 21,
        ws_col: 60,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: the descriptor is live and size points to a valid winsize.
    assert_eq!(
        unsafe {
            libc::ioctl(
                child.master.as_ref().unwrap().as_raw_fd(),
                libc::TIOCSWINSZ,
                &size,
            )
        },
        0
    );
    frame(&mut child);
    child.master.as_mut().unwrap().write_all(b"q").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // macOS drains terminal output during shutdown. Keep consuming the
        // final screen/restore sequences while waiting for the process to exit.
        let mut bytes = [0; 16384];
        let _ = child.master.as_mut().unwrap().read(&mut bytes);
        if let Some(status) = child.child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(Instant::now() < deadline, "Quit key was not handled");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Linux keeps the slave usable after its session leader exits. macOS
    // revokes it, so tcgetattr cannot inspect its modes after a clean exit.
    #[cfg(target_os = "linux")]
    {
        let mut modes = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr writes a complete termios on success.
        assert_eq!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), modes.as_mut_ptr()) },
            0
        );
        let modes = unsafe { modes.assume_init() };
        assert_ne!(modes.c_lflag & libc::ICANON, 0);
        assert_ne!(modes.c_lflag & libc::ECHO, 0);
    }
}

#[test]
fn osc72_capability_reply_and_internal_drop_copy() {
    use base64::Engine as _;
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let files = tmp.path().join("files");
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&files).unwrap();
    let folder = files.join("folder");
    let source = files.join("source.txt");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&source, b"dragged").unwrap();
    let (master, slave) = pty();
    let mut command = Command::new(env!("CARGO_BIN_EXE_starfold"));
    command
        .arg(&files)
        .env("STARFOLD_DIR", &config)
        .env("STARFOLD_CONFIG_DIR", &config)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    for key in [
        "TMUX",
        "TERM_PROGRAM",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "SSH_TTY",
        "SSH_CONNECTION",
    ] {
        command.env_remove(key);
    }
    // SAFETY: only async-signal-safe libc calls occur after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Running {
        child: command.spawn().unwrap(),
        master: Some(master),
    };
    let startup = frame(&mut child);
    assert!(startup
        .windows(b"\x1b]72;t=q:i=1\x1b\\".len())
        .any(|w| w == b"\x1b]72;t=q:i=1\x1b\\"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=q:i=1;\x1b\\")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        let mut bytes = [0u8; 4096];
        match child.master.as_mut().unwrap().read(&mut bytes) {
            Ok(n) => output.extend_from_slice(&bytes[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("PTY read: {e}"),
        }
        let accepting = output
            .windows(b"\x1b]72;t=a:i=1;text/uri-list\x1b\\".len())
            .any(|w| w == b"\x1b]72;t=a:i=1;text/uri-list\x1b\\");
        let remote = output
            .windows(b"\x1b]72;t=a:x=1:i=1".len())
            .any(|w| w == b"\x1b]72;t=a:x=1:i=1");
        let offering = output
            .windows(b"\x1b]72;t=o:x=1:i=1".len())
            .any(|w| w == b"\x1b]72;t=o:x=1:i=1");
        if accepting && remote && offering {
            break;
        }
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "terminal exited before enabling OSC 72"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        output
            .windows(b"\x1b]72;t=a:i=1;text/uri-list\x1b\\".len())
            .any(|w| w == b"\x1b]72;t=a:i=1;text/uri-list\x1b\\"),
        "OSC 72 capability reply did not enable drag support"
    );
    assert!(output
        .windows(b"\x1b]72;t=a:x=1:i=1".len())
        .any(|w| w == b"\x1b]72;t=a:x=1:i=1"));
    assert!(output
        .windows(b"\x1b]72;t=o:x=1:i=1".len())
        .any(|w| w == b"\x1b]72;t=o:x=1:i=1"));

    let source_uri = base64::engine::general_purpose::STANDARD_NO_PAD
        .encode(format!("file://{}\r\n", source.display()));
    let folder_uri = base64::engine::general_purpose::STANDARD_NO_PAD
        .encode(format!("file://{}/\r\n", folder.display()));
    let mut source_y = None;
    let mut folder_y = None;
    for y in 3..25 {
        let offer = format!("\x1b]72;t=o:x=10:y={y};\x1b\\");
        child
            .master
            .as_mut()
            .unwrap()
            .write_all(offer.as_bytes())
            .unwrap();
        let response = collect_for(&mut child, Duration::from_millis(50));
        if response
            .windows(source_uri.len())
            .any(|w| w == source_uri.as_bytes())
        {
            source_y = Some(y);
        }
        if response
            .windows(folder_uri.len())
            .any(|w| w == folder_uri.as_bytes())
        {
            folder_y = Some(y);
        }
    }
    let (source_y, folder_y) = (
        source_y.expect("source row did not offer a drag"),
        folder_y.expect("folder row did not offer a drag"),
    );
    // A Nautilus drop over an occupied file row must target the current
    // directory. A full pane may have no empty row at all.
    let hover_file = format!("\x1b]72;t=m:x=10:y={source_y}:o=3:i=1;text/uri-list\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(hover_file.as_bytes())
        .unwrap();
    let accepted = collect_for(&mut child, Duration::from_millis(60));
    assert!(accepted
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    let reenter =
        format!("\x1b]72;t=m:x=-1:y=-1:o=3:i=1;\x1b\\\x1b]72;t=m:x=10:y={source_y}:o=3:i=1;\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(reenter.as_bytes())
        .unwrap();
    let accepted_again = collect_for(&mut child, Duration::from_millis(60));
    assert!(accepted_again
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    let offer = format!("\x1b]72;t=o:x=10:y={source_y};\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(offer.as_bytes())
        .unwrap();
    let _ = collect_for(&mut child, Duration::from_millis(50));
    let drop = format!("\x1b]72;t=M:x=10:y={folder_y}:o=3:i=1;text/uri-list\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(drop.as_bytes())
        .unwrap();
    child.master.as_mut().unwrap().write_all(b"c").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = collect_for(&mut child, Duration::from_millis(20));
        if folder.join("source.txt").exists() {
            break;
        }
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "terminal exited before finishing drop"
        );
    }
    assert_eq!(
        std::fs::read(folder.join("source.txt")).unwrap(),
        b"dragged"
    );
    let _ = collect_for(&mut child, Duration::from_millis(100));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=e:x=4:y=0:i=1;\x1b\\")
        .unwrap();
    let external = tmp.path().join("external.txt");
    std::fs::write(&external, b"from desktop").unwrap();
    let hover = format!("\x1b]72;t=m:x=10:y={folder_y}:o=3:i=1;text/uri-list\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(hover.as_bytes())
        .unwrap();
    let accepted = collect_for(&mut child, Duration::from_millis(60));
    assert!(accepted
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(drop.as_bytes())
        .unwrap();
    child.master.as_mut().unwrap().write_all(b"c").unwrap();
    let requested = collect_for(&mut child, Duration::from_millis(60));
    assert!(requested
        .windows(b"t=r:x=1:i=1".len())
        .any(|w| w == b"t=r:x=1:i=1"));
    let uri_data = base64::engine::general_purpose::STANDARD
        .encode(format!("file://{}\r\n", external.display()));
    let data = format!("\x1b]72;t=r:x=1:m=1:i=1;{uri_data}\x1b\\\x1b]72;m=0:i=1;\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(data.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = collect_for(&mut child, Duration::from_millis(20));
        if folder.join("external.txt").exists() {
            break;
        }
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "terminal exited before completing desktop drop"
        );
    }
    assert_eq!(
        std::fs::read(folder.join("external.txt")).unwrap(),
        b"from desktop"
    );
    child.master.as_mut().unwrap().write_all(b"q").unwrap();
}

#[test]
fn osc72_remote_desktop_drop_streams_file() {
    use base64::Engine as _;

    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let files = tmp.path().join("files");
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&files).unwrap();
    let (master, slave) = pty();
    let mut command = Command::new(env!("CARGO_BIN_EXE_starfold"));
    command
        .arg(&files)
        .env("STARFOLD_DIR", &config)
        .env("STARFOLD_CONFIG_DIR", &config)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    for key in [
        "TMUX",
        "TERM_PROGRAM",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "SSH_CONNECTION",
    ] {
        command.env_remove(key);
    }
    command.env_remove("SSH_TTY");
    // SAFETY: only async-signal-safe libc calls occur after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Running {
        child: command.spawn().unwrap(),
        master: Some(master),
    };
    let startup = frame(&mut child);
    assert!(startup.windows(b"t=q:i=1".len()).any(|w| w == b"t=q:i=1"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=q:i=1;\x1b\\")
        .unwrap();
    let enabled = collect_for(&mut child, Duration::from_secs(1));
    assert!(
        enabled
            .windows(b"t=a:i=1;text/uri-list".len())
            .any(|w| w == b"t=a:i=1;text/uri-list"),
        "{}",
        String::from_utf8_lossy(&enabled)
    );

    let hover = b"\x1b]72;t=m:x=10:y=5:o=1:i=1;text/uri-list\x1b\\";
    child.master.as_mut().unwrap().write_all(hover).unwrap();
    let accepted = collect_for(&mut child, Duration::from_millis(100));
    assert!(accepted
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=M:x=10:y=5:o=1:i=1;text/uri-list\x1b\\")
        .unwrap();
    let requested = collect_for(&mut child, Duration::from_millis(100));
    assert!(requested
        .windows(b"t=r:x=1:i=1".len())
        .any(|w| w == b"t=r:x=1:i=1"));

    let uri = base64::engine::general_purpose::STANDARD_NO_PAD
        .encode("file:///Users/test/Desktop/from%20Mac.txt\r\n");
    // Kitty marks the nonempty final chunk m=0, then sends an empty end
    // marker. Starting the file request on the first m=0 misroutes that
    // marker as file data and aborts the transfer.
    let reply = format!("\x1b]72;t=r:x=1:m=0:X=1:i=1;{uri}\x1b\\\x1b]72;m=0:i=1;\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(reply.as_bytes())
        .unwrap();
    let requested_file = collect_for(&mut child, Duration::from_millis(100));
    assert!(requested_file
        .windows(b"t=r:x=1:y=1:i=1".len())
        .any(|w| w == b"t=r:x=1:y=1:i=1"));
    let data = base64::engine::general_purpose::STANDARD_NO_PAD.encode(b"from the Mac");
    let reply = format!("\x1b]72;t=r:x=1:y=1:m=0:i=1;{data}\x1b\\\x1b]72;m=0:i=1;\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(reply.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !files.join("from Mac.txt").exists() {
        let _ = collect_for(&mut child, Duration::from_millis(20));
        assert!(child.child.try_wait().unwrap().is_none());
    }
    assert_eq!(
        std::fs::read(files.join("from Mac.txt")).unwrap(),
        b"from the Mac"
    );
    child.master.as_mut().unwrap().write_all(b"q").unwrap();
}

#[test]
fn osc72_commander_drag_between_panes() {
    use base64::Engine as _;

    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    for path in [&config, &left, &right] {
        std::fs::create_dir(path).unwrap();
    }
    // Force a URI length that would need "==" padding with standard base64.
    // Kitty's streaming decoder rejects that padding in pre-sent drag data.
    let source_name = (0..3)
        .map(|n| format!("pane-transfer{}.txt", "x".repeat(n)))
        .find(|name| format!("file://{}\r\n", left.join(name).display()).len() % 3 == 1)
        .unwrap();
    let source = left.join(&source_name);
    std::fs::write(&source, b"commander drag").unwrap();
    std::fs::write(
        config.join("session.toml"),
        format!(
            "commander = true\ncommander_left = '{}'\ncommander_right = '{}'\ncommander_active = 0\n",
            left.display(),
            right.display()
        ),
    )
    .unwrap();

    let (master, slave) = pty();
    let mut command = Command::new(env!("CARGO_BIN_EXE_starfold"));
    command
        .arg(&left)
        .env("STARFOLD_DIR", &config)
        .env("STARFOLD_CONFIG_DIR", &config)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    for key in [
        "TMUX",
        "TERM_PROGRAM",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "SSH_TTY",
        "SSH_CONNECTION",
    ] {
        command.env_remove(key);
    }
    // SAFETY: only async-signal-safe libc calls occur after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Running {
        child: command.spawn().unwrap(),
        master: Some(master),
    };
    let startup = frame(&mut child);
    assert!(startup.windows(b"t=q:i=1".len()).any(|w| w == b"t=q:i=1"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=q:i=1;\x1b\\")
        .unwrap();
    let enabled = collect_for(&mut child, Duration::from_millis(100));
    assert!(enabled
        .windows(b"t=o:x=1:i=1".len())
        .any(|w| w == b"t=o:x=1:i=1"));

    let source_uri = base64::engine::general_purpose::STANDARD_NO_PAD
        .encode(format!("file://{}\r\n", source.display()));
    assert_eq!(source_uri.len() % 4, 2);
    let mut source_y = None;
    for y in 3..25 {
        // Kitty's first source gesture has no client ID yet.
        let offer = format!("\x1b]72;t=o:x=10:y={y};\x1b\\");
        child
            .master
            .as_mut()
            .unwrap()
            .write_all(offer.as_bytes())
            .unwrap();
        let response = collect_for(&mut child, Duration::from_millis(40));
        if response
            .windows(source_uri.len())
            .any(|w| w == source_uri.as_bytes())
        {
            source_y = Some(y);
            break;
        }
    }
    let source_y = source_y.expect("left pane file did not offer a drag");
    let offer = format!("\x1b]72;t=o:x=10:y={source_y};\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(offer.as_bytes())
        .unwrap();
    let _ = collect_for(&mut child, Duration::from_millis(40));
    let hover = "\x1b]72;t=m:x=60:y=5:o=3:i=1;text/uri-list\x1b\\";
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(hover.as_bytes())
        .unwrap();
    let accepted = collect_for(&mut child, Duration::from_millis(60));
    assert!(accepted
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    let drop = "\x1b]72;t=M:x=60:y=5:o=3:i=1;text/uri-list\x1b\\";
    // Cancel one drop, as a user may do when checking the offered action,
    // then repeat the gesture. The next drag must build a fresh source offer.
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(drop.as_bytes())
        .unwrap();
    let _ = collect_for(&mut child, Duration::from_millis(40));
    child.master.as_mut().unwrap().write_all(b"\x1b").unwrap();
    let cancelled = collect_for(&mut child, Duration::from_millis(40));
    assert!(cancelled
        .windows(b"t=r:o=0:i=1".len())
        .any(|w| w == b"t=r:o=0:i=1"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(b"\x1b]72;t=e:x=4:y=1:i=1;\x1b\\")
        .unwrap();
    let second_offer = format!("\x1b]72;t=o:x=10:y={source_y}:i=1;\x1b\\");
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(second_offer.as_bytes())
        .unwrap();
    let response = collect_for(&mut child, Duration::from_millis(60));
    assert!(response
        .windows(source_uri.len())
        .any(|w| w == source_uri.as_bytes()));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(hover.as_bytes())
        .unwrap();
    let accepted_again = collect_for(&mut child, Duration::from_millis(60));
    assert!(accepted_again
        .windows(b"t=m:o=1:i=1;text/uri-list".len())
        .any(|w| w == b"t=m:o=1:i=1;text/uri-list"));
    child
        .master
        .as_mut()
        .unwrap()
        .write_all(drop.as_bytes())
        .unwrap();
    child.master.as_mut().unwrap().write_all(b"c").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = collect_for(&mut child, Duration::from_millis(20));
        if right.join(&source_name).exists() {
            break;
        }
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "terminal exited before finishing Commander drop"
        );
    }
    assert_eq!(
        std::fs::read(right.join(&source_name)).unwrap(),
        b"commander drag"
    );
    child.master.as_mut().unwrap().write_all(b"q").unwrap();
}
