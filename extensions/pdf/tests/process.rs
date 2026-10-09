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
    assert!(presentation
        .fields
        .iter()
        .any(|(label, value)| label == "View" && value.contains("70%")));
    assert_eq!(bytes.len(), 40000);
    assert_eq!(&bytes[..4], &[0, 0, 0, 255]);
    let (reply, bytes) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "n".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    assert_eq!(presentation.raster.unwrap().page, 2);
    let middle = (50 * 100 + 50) * 4;
    assert!(bytes[middle + 2] > 0);
    assert_eq!(
        &bytes[(20 * 100 + 14) * 4..(20 * 100 + 15) * 4],
        &[0, 0, 0, 255]
    );
    assert!(bytes[(20 * 100 + 16) * 4 + 2] > 0);
    assert!(bytes[(20 * 100 + 83) * 4 + 2] > 0);
    assert_eq!(
        &bytes[(20 * 100 + 86) * 4..(20 * 100 + 87) * 4],
        &[0, 0, 0, 255]
    );
    let (_, cached) = helper.request(p::Request::Input {
        input: p::Input::Page { page: 2 },
    });
    assert_eq!(cached, bytes);
    helper.request(p::Request::Input {
        input: p::Input::Key { key: "w".into() },
    });
    // A wide, shallow pane should show a readable width-sized page without
    // white gutters. The raster limit preserves the pane's aspect ratio.
    let (reply, bytes) = helper.request(p::Request::Input {
        input: p::Input::Viewport {
            viewport: p::Viewport {
                width: 400,
                height: 100,
                background: "#102030".into(),
                ..Default::default()
            },
        },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    let raster = presentation.raster.unwrap();
    assert_eq!((raster.width, raster.height), (200, 50));
    assert_eq!(bytes[0], 0); // page fills the left edge
    let (_, fit_page) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "0".into() },
    });
    assert_eq!(&fit_page[..4], &[16, 32, 48, 255]); // themed canvas outside the page
    let (_, light_canvas) = helper.request(p::Request::Input {
        input: p::Input::Viewport {
            viewport: p::Viewport {
                width: 400,
                height: 100,
                background: "#e0e1e2".into(),
                ..Default::default()
            },
        },
    });
    assert_eq!(&light_canvas[..4], &[224, 225, 226, 255]);
    let center = (25 * 200 + 100) * 4;
    assert_eq!(
        &light_canvas[center..center + 4],
        &fit_page[center..center + 4]
    );
    let (_, fit_width) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "w".into() },
    });
    assert_eq!(fit_width, bytes);
    let mut blank = Document::load(&path).unwrap();
    let first_page = *blank.get_pages().get(&1).unwrap();
    blank
        .get_dictionary_mut(first_page)
        .unwrap()
        .remove(b"Contents");
    blank.save(&path).unwrap();
    let (reply, bytes) = helper.request(p::Request::Open {
        path: path.as_os_str().as_bytes().to_vec(),
        limits: p::Limits {
            text_bytes: 4096,
            cache_bytes: 4096,
            image_dimension: 4096,
        },
        viewport: p::Viewport {
            width: 2400,
            height: 200,
            ..Default::default()
        },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    presentation.validate(&bytes).unwrap();
    assert_eq!(
        &bytes[(100 * 2400 + 1200) * 4..(100 * 2400 + 1201) * 4],
        &[255, 255, 255, 255],
        "unpainted PDF paper must stay white"
    );
    let raster = presentation.raster.unwrap();
    assert_eq!(
        (raster.width, raster.height),
        (2400, 200),
        "large panes need display-sized document rasters"
    );
    helper.request(p::Request::Input {
        input: p::Input::Key { key: "w".into() },
    });
    let (_, rounded) = helper.request(p::Request::Input {
        input: p::Input::Viewport {
            viewport: p::Viewport {
                width: 2400,
                height: 200,
                corner_radius: 24,
                background: "#102030".into(),
                ..Default::default()
            },
        },
    });
    assert_eq!(
        &rounded[..4],
        &[16, 32, 48, 255],
        "page corner reveals the theme canvas"
    );
    assert_eq!(
        &rounded[100 * 4..101 * 4],
        &[255, 255, 255, 255],
        "page edge remains white beyond the corner"
    );
    let (_, square) = helper.request(p::Request::Input {
        input: p::Input::Viewport {
            viewport: p::Viewport {
                width: 2400,
                height: 200,
                corner_radius: 0,
                background: "#102030".into(),
                ..Default::default()
            },
        },
    });
    assert_eq!(
        &square[..4],
        &[255, 255, 255, 255],
        "rigid setting restores square paper corners"
    );
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

#[test]
fn font_extraction_panic_keeps_rendering_and_page_navigation_alive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("font-regression.pdf");
    pdf(&path);
    let mut document = Document::load(&path).unwrap();
    let second = *document.get_pages().get(&2).unwrap();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats", "Encoding" => "WinAnsiEncoding"
    });
    let text = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
            Operation::new("Td", vec![10.into(), 10.into()]),
            Operation::new("Tj", vec![Object::string_literal("font regression")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content = document.add_object(Stream::new(dictionary! {}, text.encode().unwrap()));
    let page = document.get_dictionary_mut(second).unwrap();
    page.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    page.set("Contents", content);
    document.save(&path).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_starfold-preview-pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut helper = Helper { child, seq: 0 };
    helper.request(p::Request::Hello {
        version: p::VERSION,
        capabilities: vec!["raster".into()],
    });
    helper.request(p::Request::Open {
        path: path.as_os_str().as_bytes().to_vec(),
        limits: p::Limits {
            text_bytes: 4096,
            cache_bytes: 4096,
            image_dimension: 200,
        },
        viewport: p::Viewport {
            width: 100,
            height: 100,
            corner_radius: 24,
            ..Default::default()
        },
    });
    let (reply, bytes) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "n".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(presentation.raster.as_ref().unwrap().page, 2);
    assert!(presentation
        .notice
        .as_ref()
        .is_some_and(|notice| notice.contains("Text extraction unavailable")));
    presentation.validate(&bytes).unwrap();
    assert!(!bytes.is_empty());
    let (reply, _) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "t".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    assert!(presentation.raster.is_none());
    helper.request(p::Request::Input {
        input: p::Input::Key { key: "t".into() },
    });
    let (reply, _) = helper.request(p::Request::Input {
        input: p::Input::Key { key: "n".into() },
    });
    let p::Reply::Content { presentation } = reply else {
        panic!()
    };
    assert_eq!(presentation.raster.unwrap().page, 3);
    helper.request(p::Request::Close);
    assert!(helper.child.wait().unwrap().success());
}
