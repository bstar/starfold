use starfold_preview_protocol as p;
use std::{
    io::Write,
    os::unix::ffi::OsStrExt,
    process::{Command, Stdio},
};
#[test]
fn poster_and_actions_use_existing_media_services() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.y4m");
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(b"YUV4MPEG2 W32 H24 F25:1 Ip A1:1 C420jpeg\nFRAME\n")
        .unwrap();
    file.write_all(&vec![128; 32 * 24 * 3 / 2]).unwrap();
    drop(file);
    let mut child = Command::new(env!("CARGO_BIN_EXE_starfold-preview-video"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut request = |message, sequence| {
        p::write_frame(
            child.stdin.as_mut().unwrap(),
            &p::Envelope {
                session: 1,
                generation: 2,
                sequence,
                message,
            },
            &[],
        )
        .unwrap();
        p::read_frame::<p::Envelope<p::Reply>>(child.stdout.as_mut().unwrap())
            .unwrap()
            .unwrap()
    };
    request(
        p::Request::Hello {
            version: 1,
            capabilities: vec!["media".into()],
        },
        1,
    );
    let (reply, bytes) = request(
        p::Request::Open {
            path: path.as_os_str().as_bytes().to_vec(),
            limits: p::Limits {
                text_bytes: 4096,
                cache_bytes: 4096,
                image_dimension: 4096,
            },
            viewport: Default::default(),
        },
        2,
    );
    let p::Reply::Content { presentation } = reply.message else {
        panic!("{reply:?}")
    };
    presentation.validate(&bytes).unwrap();
    assert_eq!(presentation.media.unwrap().width, 32);
    let (reply, _) = request(
        p::Request::Input {
            input: p::Input::Key { key: "p".into() },
        },
        3,
    );
    let p::Reply::Content { presentation } = reply.message else {
        panic!()
    };
    assert_eq!(presentation.actions, vec![p::MediaAction::PlayPause]);
    request(p::Request::Close, 4);
    assert!(child.wait().unwrap().success());
}
