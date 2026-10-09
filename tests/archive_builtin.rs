//! The installed FOLD executable provides archives without an adjacent helper.
use starfold_archive_protocol::{receive, send, Reply, Request};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

#[test]
fn standalone_fold_provides_archive_index_and_exact_member_stream() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("starfold");
    std::fs::copy(env!("CARGO_BIN_EXE_starfold"), &executable).unwrap();
    assert!(!temp.path().join("starfold-archive").exists());
    let source = temp.path().join("sample.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
    zip.start_file("hello.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"archive inside FOLD").unwrap();
    zip.finish().unwrap();
    for request in [
        Request::List {
            source: source.clone(),
            limit: 100,
            password: None,
        },
        Request::Read {
            source,
            index: 0,
            password: None,
        },
    ] {
        let mut child = Command::new(&executable)
            .arg("--archive-extension-stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = child.stdout.take().unwrap();
        assert!(matches!(
            receive::<Reply>(&mut output).unwrap(),
            Reply::Hello { version: 1, .. }
        ));
        let reading = matches!(request, Request::Read { .. });
        send(&mut child.stdin.take().unwrap(), &request).unwrap();
        let mut bytes = vec![];
        let mut indexed = false;
        loop {
            match receive::<Reply>(&mut output).unwrap() {
                Reply::Entries { entries, partial } => {
                    assert!(!partial);
                    assert_eq!(entries[0].name, "hello.txt");
                    indexed = true;
                }
                Reply::Data { length } => {
                    let mut chunk = vec![0; length as usize];
                    output.read_exact(&mut chunk).unwrap();
                    bytes.extend(chunk);
                }
                Reply::Progress(_) => {}
                Reply::Done => break,
                other => panic!("Unexpected archive response: {other:?}"),
            }
        }
        if reading {
            assert_eq!(bytes, b"archive inside FOLD");
        } else {
            assert!(indexed);
        }
        assert!(child.wait().unwrap().success());
    }
}
