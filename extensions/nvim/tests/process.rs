use starfold_preview_protocol as p;
use std::{
    io::{BufReader, Write},
    os::unix::ffi::OsStrExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
struct Helper {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Helper {
    fn request(&mut self, message: p::Request) -> p::Reply {
        self.sequence += 1;
        p::write_frame(
            &mut self.input,
            &p::Envelope {
                session: 9,
                generation: 3,
                sequence: self.sequence,
                message,
            },
            &[],
        )
        .unwrap();
        self.input.flush().unwrap();
        let (reply, bytes) = p::read_frame::<p::Envelope<p::Reply>>(&mut self.output)
            .unwrap()
            .unwrap();
        assert_eq!(
            (reply.session, reply.generation, reply.sequence),
            (9, 3, self.sequence)
        );
        assert!(bytes.is_empty());
        if let p::Reply::Content { presentation } = &reply.message {
            presentation.validate(&bytes).unwrap();
        }
        reply.message
    }
    fn input(&mut self, input: p::Input) -> Box<p::Presentation> {
        match self.request(p::Request::Input { input }) {
            p::Reply::Content { presentation } => presentation,
            other => panic!("Unexpected: {other:?}"),
        }
    }
    fn key(&mut self, key: &str) -> Box<p::Presentation> {
        self.input(p::Input::Key { key: key.into() })
    }
    fn command(&mut self, command: &str) -> Box<p::Presentation> {
        self.key(":");
        for c in command.chars() {
            self.key(&c.to_string());
        }
        self.key("enter")
    }
}
#[test]
fn embedded_editor_inserts_saves_resizes_refuses_dirty_quit_and_discards_explicitly() {
    exercise_editor(false);
}

#[test]
fn embedded_editor_loads_user_configuration_and_saves() {
    exercise_editor(true);
}

fn exercise_editor(user_config: bool) {
    let Ok(nvim) = std::env::var("STARFOLD_TEST_NVIM") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spaces ' λ.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_starfold-preview-nvim"));
    if !user_config {
        let config = dir.path().join("config/nvim");
        std::fs::create_dir_all(&config).unwrap();
        let counter = dir.path().join("starts");
        std::fs::write(
            config.join("init.lua"),
            format!(
                "vim.fn.writefile({{'started'}}, {:?}, 'a')",
                counter.to_str().unwrap()
            ),
        )
        .unwrap();
        command.env("XDG_CONFIG_HOME", dir.path().join("config"));
    }
    let mut child = command
        .arg(nvim)
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    let mut h = Helper {
        child,
        input,
        output,
        sequence: 0,
    };
    assert!(matches!(
        h.request(p::Request::Hello {
            version: p::VERSION,
            capabilities: vec!["surfaces".into(), "editor".into()]
        }),
        p::Reply::Hello { .. }
    ));
    let open = h.request(p::Request::Open {
        path: path.as_os_str().as_bytes().to_vec(),
        limits: p::Limits {
            text_bytes: 4096,
            cache_bytes: 4096,
            image_dimension: 4096,
        },
        viewport: Default::default(),
    });
    assert!(
        matches!(open,p::Reply::Content {presentation} if presentation.interactive && presentation.surface.is_some())
    );
    if !user_config {
        // Exercise actual RPC mouse selection and Neovim's internal popup.
        h.input(p::Input::Viewport {
            viewport: p::Viewport {
                cells: Some([80, 24]),
                ..Default::default()
            },
        });
        let pointer = |action: &str, button, x| p::Input::Pointer {
            action: action.into(),
            button,
            x,
            y: 0,
        };
        h.input(pointer("down", 0, 0));
        h.input(pointer("drag", 0, 2));
        h.input(pointer("up", 0, 2));
        let selection = h.input(p::Input::Action {
            action: "refresh".into(),
        });
        assert!(
            selection
                .fields
                .iter()
                .any(|(_, value)| value.contains("visual")),
            "mouse drag did not select: {:?}",
            selection.fields
        );
        h.key("escape");
        h.input(pointer("down", 1, 1));
        let popup = h.input(p::Input::Action {
            action: "refresh".into(),
        });
        assert!(
            popup.pages[0].text.contains("Paste"),
            "right-click popup missing: {}",
            popup.pages[0].text
        );
        h.key("escape");
        h.key("g");
        h.key("g");
        h.key("0");
    }
    h.key("i");
    let edited = h.input(p::Input::Paste {
        text: "edited ".into(),
    });
    assert!(edited.interactive);
    h.key("escape");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let modified = h.input(p::Input::Action {
        action: "refresh".into(),
    });
    assert!(modified.modified, "expected modified editor: {modified:?}");
    if !user_config {
        let refused = h.command("q");
        assert!(refused.interactive);
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\nthree\n");
    let saved = h.input(p::Input::Action {
        action: "editor-save".into(),
    });
    assert!(
        saved.interactive,
        "save unexpectedly closed editor: {saved:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "edited one\ntwo\nthree\n"
    );
    let resized = h.input(p::Input::Viewport {
        viewport: p::Viewport {
            cells: None,
            width: 480,
            height: 240,
            foreground: "#123456".into(),
            background: "#abcdef".into(),
        },
    });
    assert!(resized.surface.is_some(), "resize failed: {resized:?}");
    let cells = h.input(p::Input::Viewport {
        viewport: p::Viewport {
            cells: Some([60, 12]),
            ..Default::default()
        },
    });
    let grid = cells.cells.as_ref().expect("styled editor grid");
    assert_eq!((grid.columns, grid.rows), (60, 12));
    grid.validate().unwrap();
    assert!(grid.cursor.is_some());
    assert!(!cells.modified);
    let restored = h.input(p::Input::Viewport {
        viewport: Default::default(),
    });
    assert!(restored.interactive && !restored.modified);

    let next_path = dir.path().join("next.txt");
    std::fs::write(&next_path, "second file\n").unwrap();
    for target in [&next_path, &path] {
        let reply = h.request(p::Request::Open {
            path: target.as_os_str().as_bytes().to_vec(),
            limits: p::Limits {
                text_bytes: 4096,
                cache_bytes: 4096,
                image_dimension: 4096,
            },
            viewport: Default::default(),
        });
        assert!(matches!(reply, p::Reply::Content { presentation } if presentation.interactive));
    }
    let count = dir.path().join("buffer-count");
    h.command(&format!("call writefile([string(len(filter(nvim_list_bufs(), 'buflisted(v:val) && getbufvar(v:val, \"&buftype\") == \"\"')))], '{}')", count.display()));
    assert_eq!(
        std::fs::read_to_string(&count).unwrap().trim(),
        "1",
        "Clean previews must not accumulate buffer tabs"
    );
    if !user_config {
        assert_eq!(
            std::fs::read_to_string(dir.path().join("starts")).unwrap(),
            "started\n",
            "Reopening must reuse Neovim and load the config only once"
        );
    }
    h.key("i");
    h.input(p::Input::Paste {
        text: "discarded ".into(),
    });
    h.key("escape");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let closed = h.command("q!");
    assert!(!closed.interactive);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "edited one\ntwo\nthree\n"
    );
    assert!(matches!(h.request(p::Request::Close), p::Reply::Closed));
    assert!(h.child.wait().unwrap().success());
}
