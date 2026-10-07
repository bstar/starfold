use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use starfold_preview_protocol as p;
use std::{
    os::unix::ffi::OsStrExt,
    process::{Child, Command, Stdio},
};
struct Helper {
    child: Child,
    seq: u64,
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Helper {
    fn request(&mut self, message: p::Request) -> (p::Reply, Vec<u8>) {
        self.seq += 1;
        p::write_frame(
            self.child.stdin.as_mut().unwrap(),
            &p::Envelope {
                session: 7,
                generation: 11,
                sequence: self.seq,
                message,
            },
            &[],
        )
        .unwrap();
        let (reply, bytes) =
            p::read_frame::<p::Envelope<p::Reply>>(self.child.stdout.as_mut().unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(
            (reply.session, reply.generation, reply.sequence),
            (7, 11, self.seq)
        );
        (reply.message, bytes)
    }
}
fn pdf(path: &std::path::Path) {
    let mut doc = Document::with_version("1.7");
    let root = doc.new_object_id();
    let mut kids = vec![];
    for page in 0..3 {
        let content = Content {
            operations: vec![
                Operation::new("rg", vec![0.into(), 0.into(), (page as f32 / 3.0).into()]),
                Operation::new("re", vec![0.into(), 0.into(), 100.into(), 100.into()]),
                Operation::new("f", vec![]),
            ],
        };
        let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let id = doc.add_object(dictionary! {"Type"=>"Page","Parent"=>root,"Contents"=>stream});
        kids.push(Object::Reference(id));
    }
    doc.objects.insert(root,dictionary!{"Type"=>"Pages","Kids"=>kids,"Count"=>3,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}.into());
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}
#[test]
fn renders_pages_zoom_text_and_close_over_real_process() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(target_os = "linux")]
    let path = dir
        .path()
        .join(std::ffi::OsStr::from_bytes(b"book-\xff.pdf"));
    #[cfg(not(target_os = "linux"))]
    let path = dir.path().join("book-é.pdf");
    pdf(&path);
    let child = Command::new(env!("CARGO_BIN_EXE_starfold-preview-pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut helper = Helper { child, seq: 0 };
    let (reply, _) = helper.request(p::Request::Hello {
        version: p::VERSION,
        capabilities: vec!["raster".into()],
    });
    assert!(matches!(reply, p::Reply::Hello { version: 1, .. }));
    let (reply, bytes) = helper.request(p::Request::Open {
        path: path.as_os_str().as_bytes().to_vec(),
        limits: p::Limits {
            text_bytes: 4096,
            cache_bytes: 4096,
            image_dimension: 200,
        },
        viewport: p::Viewport {
            width: 100,
            height: 100,
            ..Default::default()
        },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!("{reply:?}")
    };
    presentation.validate(&bytes).unwrap();
    assert_eq!(presentation.total_pages, Some(3));
    assert_eq!(bytes.len(), 40000);
    assert_eq!(&bytes[..4], &[0, 0, 0, 255]);
    let (reply, bytes) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "n".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    assert_eq!(presentation.raster.unwrap().page, 2);
    assert!(bytes[2] > 0);
    let (_, cached) = helper.request(p::Request::Input {
        input: p::Input::Page { page: 2 },
    });
    assert_eq!(cached, bytes);
    let (reply, bytes) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "t".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    assert!(presentation.raster.is_none());
    assert!(!presentation.keys.iter().any(|key| key == "down"));
    assert!(bytes.is_empty());
    let (reply, _) = helper.request(p::Request::Close);
    assert!(matches!(reply, p::Reply::Closed));
    assert!(helper.child.wait().unwrap().success());
}
#[test]
fn version_mismatch_exits_without_opening_file() {
    let child = Command::new(env!("CARGO_BIN_EXE_starfold-preview-pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut helper = Helper { child, seq: 0 };
    p::write_frame(
        helper.child.stdin.as_mut().unwrap(),
        &p::Envelope {
            session: 1,
            generation: 1,
            sequence: 1,
            message: p::Request::Hello {
                version: 999,
                capabilities: vec![],
            },
        },
        &[],
    )
    .unwrap();
    assert!(
        p::read_frame::<p::Envelope<p::Reply>>(helper.child.stdout.as_mut().unwrap())
            .unwrap()
            .is_none()
    );
    assert!(!helper.child.wait().unwrap().success());
}

#[test]
fn malformed_pdf_reports_error_without_losing_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.pdf");
    std::fs::write(&path, b"%PDF-1.7 malformed input").unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_starfold-preview-pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut helper = Helper { child, seq: 0 };
    helper.request(p::Request::Hello {
        version: 1,
        capabilities: vec![],
    });
    let (reply, bytes) = helper.request(p::Request::Open {
        path: path.as_os_str().as_bytes().to_vec(),
        limits: p::Limits {
            text_bytes: 4096,
            cache_bytes: 4096,
            image_dimension: 200,
        },
        viewport: Default::default(),
    });
    assert!(matches!(reply, p::Reply::Error { .. }));
    assert!(bytes.is_empty());
    let (reply, _) = helper.request(p::Request::Close);
    assert!(matches!(reply, p::Reply::Closed));
    assert!(helper.child.wait().unwrap().success());
}
