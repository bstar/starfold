//! Real-process coverage of the private preview protocol. No installed helpers.
use starfold_preview_protocol as wire;
use std::{
    os::unix::ffi::OsStrExt,
    process::{Command, Stdio},
    time::Duration,
};
fn pdf(path: &std::path::Path) {
    use lopdf::{
        content::{Content, Operation},
        dictionary, Object, Stream,
    };
    let mut doc = lopdf::Document::with_version("1.5");
    let root = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let resources = doc.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font}});
    let mut kids = vec![];
    for n in 1..=8 {
        let c = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(format!("Page {n} text"))]),
                Operation::new("ET", vec![]),
            ],
        };
        let content = doc.add_object(Stream::new(dictionary! {}, c.encode().unwrap()));
        let page = doc.add_object(dictionary! {"Type"=>"Page","Parent"=>root,"Contents"=>content});
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(root,dictionary!{"Type"=>"Pages","Kids"=>kids,"Count"=>8,"Resources"=>resources,"MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()]}.into());
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}
#[test]
fn pdf_extension_retains_document_and_exits_on_eof_without_creating_config() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.pdf");
    pdf(&path);
    let config = tmp.path().join("config");
    let helper = std::path::Path::new(env!("CARGO_BIN_EXE_starfold"))
        .parent()
        .unwrap()
        .join("starfold-preview-pdf");
    let mut child = Command::new(helper)
        .env("STARFOLD_DIR", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        while let Some((reply, bytes)) =
            wire::read_frame::<wire::Envelope<wire::Reply>>(&mut stdout).unwrap()
        {
            if tx.send((reply, bytes)).is_err() {
                break;
            }
        }
    });
    let requests = [
        wire::Request::Hello {
            version: 1,
            capabilities: vec!["raster".into()],
        },
        wire::Request::Open {
            path: path.as_os_str().as_bytes().to_vec(),
            limits: wire::Limits {
                text_bytes: 262144,
                cache_bytes: 33554432,
                image_dimension: 4096,
            },
            viewport: Default::default(),
        },
        wire::Request::Input {
            input: wire::Input::Page { page: 4 },
        },
        wire::Request::Input {
            input: wire::Input::Page { page: 7 },
        },
    ];
    for (index, message) in requests.into_iter().enumerate() {
        wire::write_frame(
            &mut input,
            &wire::Envelope {
                session: 5,
                generation: 6,
                sequence: index as u64,
                message,
            },
            &[],
        )
        .unwrap();
        let (reply, bytes) = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(packet) => packet,
            Err(e) => {
                let _ = child.kill();
                panic!("extension did not reply: {e}");
            }
        };
        assert_eq!(
            (reply.session, reply.generation, reply.sequence),
            (5, 6, index as u64)
        );
        if index == 0 {
            assert!(matches!(
                reply.message,
                wire::Reply::Hello { version: 1, .. }
            ));
            continue;
        }
        let wire::Reply::Content { presentation } = reply.message else {
            panic!("{reply:?}")
        };
        presentation.validate(&bytes).unwrap();
        let page = [1, 4, 7][index - 1];
        assert_eq!(presentation.kind, "PDF");
        assert_eq!(presentation.total_pages, Some(8));
        assert_eq!(presentation.pages[0].number, page);
        assert!(presentation.pages[0]
            .text
            .contains(&format!("Page {page} text")));
        assert!(presentation.raster.is_some());
        if index == 1 {
            std::fs::remove_file(&path).unwrap();
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    assert!(!config.exists());
}

#[test]
fn archive_extension_creates_an_interoperable_zip_and_extracts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("note.txt");
    std::fs::write(&source, b"A note\n").unwrap();
    let archive = tmp.path().join("result.zip");
    let run = |request: starfold_archive_protocol::Request| {
        use starfold_archive_protocol::{receive, send, Reply};
        let helper =
            std::path::Path::new(env!("CARGO_BIN_EXE_starfold")).with_file_name("starfold-archive");
        let mut child = Command::new(helper)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = child.stdout.take().unwrap();
        assert!(matches!(
            receive::<Reply>(&mut output).unwrap(),
            Reply::Hello { version: 1, .. }
        ));
        send(&mut child.stdin.take().unwrap(), &request).unwrap();
        loop {
            match receive::<Reply>(&mut output).unwrap() {
                Reply::Done => break,
                Reply::Progress(_) => {}
                other => panic!("{other:?}"),
            }
        }
        assert!(child.wait().unwrap().success());
    };
    run(starfold_archive_protocol::Request::Create {
        format: starfold_archive_protocol::Format::Zip,
        output: archive.clone(),
        items: vec![starfold_archive_protocol::Item {
            from: source,
            to: Some("note.txt".into()),
            kind: starfold_archive_protocol::ItemKind::File(7),
        }],
        options: Default::default(),
    });
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(zip.by_index(0).unwrap().name(), "note.txt");
    let dest = tmp.path().join("extracted");
    run(starfold_archive_protocol::Request::Extract {
        source: archive,
        output: dest.clone(),
        password: None,
    });
    assert_eq!(std::fs::read(dest.join("note.txt")).unwrap(), b"A note\n");
}
