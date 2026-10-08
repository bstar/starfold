use starfold_archive_protocol::{
    receive, send, Entry, Format, Item, ItemKind, Options, Preset, Reply, Request, VERSION,
};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};
fn request(request: Request) -> anyhow::Result<(Vec<Entry>, Vec<u8>)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_starfold-archive"))
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let result = (|| {
        let mut output = child.stdout.take().unwrap();
        assert!(matches!(
            receive::<Reply>(&mut output)?,
            Reply::Hello {
                version: VERSION,
                ..
            }
        ));
        send(&mut child.stdin.take().unwrap(), &request)?;
        let mut entries = vec![];
        let mut data = vec![];
        loop {
            match receive::<Reply>(&mut output)? {
                Reply::Entries {
                    entries: rows,
                    partial,
                } => {
                    assert!(!partial);
                    entries = rows;
                }
                Reply::Data { length } => {
                    assert!(length <= 256 * 1024);
                    let start = data.len();
                    data.resize(start + length as usize, 0);
                    output.read_exact(&mut data[start..])?;
                }
                Reply::Progress(_) => {}
                Reply::Done => break,
                Reply::Error { message } => anyhow::bail!(message),
                _ => panic!("unexpected handshake"),
            }
        }
        Ok((entries, data))
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}
fn create(root: &Path, format: Format, options: Options, name: &str) -> std::path::PathBuf {
    let input = root.join("source.txt");
    std::fs::write(&input, b"extension member data").unwrap();
    let output = root.join(name);
    request(Request::Create {
        format,
        output: output.clone(),
        items: vec![Item {
            from: input,
            to: Some("folder/source.txt".into()),
            kind: ItemKind::File(21),
        }],
        options,
    })
    .unwrap();
    output
}
#[test]
fn framed_create_list_exact_read_extract_and_test() {
    let temp = tempfile::tempdir().unwrap();
    for (format, name) in [
        (Format::Zip, "a.zip"),
        (Format::Tar, "a.tar"),
        (Format::TarGz, "a.tar.gz"),
        (Format::TarZst, "a.tar.zst"),
        (Format::SevenZip, "a.7z"),
    ] {
        let source = create(temp.path(), format, Options::default(), name);
        let (entries, _) = request(Request::List {
            source: source.clone(),
            limit: 100,
            password: None,
        })
        .unwrap();
        let index = entries
            .iter()
            .position(|e| e.name == "folder/source.txt")
            .unwrap();
        let (_, data) = request(Request::Read {
            source: source.clone(),
            index,
            password: None,
        })
        .unwrap();
        assert_eq!(data, b"extension member data");
        request(Request::Test {
            source: source.clone(),
            password: None,
        })
        .unwrap();
        let output = temp.path().join(format!("out-{name}"));
        request(Request::Extract {
            source,
            output: output.clone(),
            password: None,
        })
        .unwrap();
        assert_eq!(
            std::fs::read(output.join("folder/source.txt")).unwrap(),
            data
        );
    }
}
#[test]
fn store_and_aes_passwords_and_split_volumes() {
    let temp = tempfile::tempdir().unwrap();
    let source = create(
        temp.path(),
        Format::Zip,
        Options {
            preset: Preset::Store,
            ..Options::default()
        },
        "store.zip",
    );
    request(Request::Read {
        source,
        index: 0,
        password: None,
    })
    .unwrap();
    let source = create(
        temp.path(),
        Format::Zip,
        Options {
            password: Some("secret-password".into()),
            ..Options::default()
        },
        "encrypted.zip",
    );
    let index = request(Request::List {
        source: source.clone(),
        limit: 100,
        password: None,
    })
    .unwrap()
    .0
    .iter()
    .position(|e| e.name == "folder/source.txt")
    .unwrap();
    assert!(request(Request::Read {
        source: source.clone(),
        index,
        password: Some("wrong".into())
    })
    .is_err());
    assert_eq!(
        request(Request::Read {
            source,
            index,
            password: Some("secret-password".into())
        })
        .unwrap()
        .1,
        b"extension member data"
    );
    let source = create(
        temp.path(),
        Format::SevenZip,
        Options {
            volume_bytes: Some(1024 * 1024),
            ..Options::default()
        },
        "split.7z",
    );
    let volume = source.with_file_name("split.7z.001");
    assert!(volume.exists());
    request(Request::Test {
        source: volume,
        password: None,
    })
    .unwrap();
}
#[test]
fn rebuild_keeps_original_private_until_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = create(temp.path(), Format::Zip, Options::default(), "edit.zip");
    let before = std::fs::read(&source).unwrap();
    let output = temp.path().join("new.zip");
    request(Request::Rebuild {
        source: source.clone(),
        output: output.clone(),
        changes: vec![starfold_archive_protocol::Change {
            index: 0,
            name: Some("renamed.txt".into()),
        }],
        additions: vec![],
    })
    .unwrap();
    assert_eq!(std::fs::read(source).unwrap(), before);
    let (entries, _) = request(Request::List {
        source: output,
        limit: 100,
        password: None,
    })
    .unwrap();
    assert_eq!(entries[0].name, "renamed.txt");
}

#[test]
fn seven_zip_duplicate_names_are_read_by_exact_header_identity() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let source = temp.path().join("duplicate.7z");
    request(Request::Create {
        format: Format::SevenZip,
        output: source.clone(),
        items: vec![
            Item {
                from: first,
                to: Some("same.txt".into()),
                kind: ItemKind::File(5),
            },
            Item {
                from: second,
                to: Some("same.txt".into()),
                kind: ItemKind::File(6),
            },
        ],
        options: Default::default(),
    })
    .unwrap();
    assert_eq!(
        request(Request::Read {
            source: source.clone(),
            index: 0,
            password: None
        })
        .unwrap()
        .1,
        b"first"
    );
    assert_eq!(
        request(Request::Read {
            source,
            index: 1,
            password: None
        })
        .unwrap()
        .1,
        b"second"
    );
}

#[test]
fn content_inspection_and_encrypted_exact_member_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let source = create(
        temp.path(),
        Format::Zip,
        Options {
            password: Some("secret-password".into()),
            ..Default::default()
        },
        "encrypted.zip",
    );
    let renamed = temp.path().join("renamed.data");
    std::fs::copy(&source, &renamed).unwrap();
    let (_, bytes) = request(Request::Inspect {
        source: renamed.clone(),
    })
    .unwrap();
    let inspection: starfold_archive_protocol::Inspection = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(inspection.format, Format::Zip);
    assert!(inspection.editable && inspection.encrypted);
    let (entries, _) = request(Request::List {
        source: renamed.clone(),
        limit: 100,
        password: Some("secret-password".into()),
    })
    .unwrap();
    let index = entries.iter().position(|e| !e.directory).unwrap();
    let replacement = temp.path().join("replacement.cfg");
    std::fs::write(&replacement, b"edited=yes\n").unwrap();
    let output = temp.path().join("edited.zip");
    request(Request::RebuildEdited {
        source: renamed.clone(),
        output: output.clone(),
        changes: vec![],
        additions: vec![],
        replacements: vec![starfold_archive_protocol::Replacement {
            index,
            from: replacement.clone(),
        }],
        password: Some("secret-password".into()),
    })
    .unwrap();
    assert!(request(Request::Read {
        source: output.clone(),
        index,
        password: None
    })
    .is_err());
    assert!(request(Request::Read {
        source: output.clone(),
        index,
        password: Some("wrong".into())
    })
    .is_err());
    assert_eq!(
        request(Request::Read {
            source: output,
            index,
            password: Some("secret-password".into())
        })
        .unwrap()
        .1,
        b"edited=yes\n"
    );
    assert!(request(Request::RebuildEdited {
        source: renamed,
        output: temp.path().join("no-password.zip"),
        changes: vec![],
        additions: vec![],
        replacements: vec![starfold_archive_protocol::Replacement {
            index,
            from: replacement
        }],
        password: None
    })
    .is_err());
}

#[test]
fn read_only_save_as_conversion_creates_zip_without_changing_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = create(
        temp.path(),
        Format::TarGz,
        Options::default(),
        "readonly.tar.gz",
    );
    let original = std::fs::read(&source).unwrap();
    let output = temp.path().join("converted.zip");
    request(Request::ConvertToZip {
        source: source.clone(),
        output: output.clone(),
        password: None,
    })
    .unwrap();
    assert_eq!(std::fs::read(source).unwrap(), original);
    let (entries, _) = request(Request::List {
        source: output.clone(),
        limit: 100,
        password: None,
    })
    .unwrap();
    let i = entries
        .iter()
        .position(|e| e.name == "folder/source.txt")
        .unwrap();
    assert_eq!(
        request(Request::Read {
            source: output,
            index: i,
            password: None
        })
        .unwrap()
        .1,
        b"extension member data"
    );
}
