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
    // An ordinary error or successful exit must both give the keyboard back.
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
