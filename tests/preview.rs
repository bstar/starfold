//! Real-process coverage of the private preview protocol. No installed helpers.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
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
fn parser_process_retains_pdf_and_exits_on_eof_without_creating_config() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("book.pdf");
    pdf(&path);
    let config = tmp.path().join("config");
    let mut child = Command::new(env!("CARGO_BIN_EXE_starfold"))
        .arg("--preview-worker")
        .env("STARFOLD_DIR", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    for (page, count, next) in [(1, 3, json!(4)), (4, 3, json!(7)), (7, 2, Value::Null)] {
        let request = json!({"path":path.as_os_str().as_bytes(),"page":page,"cfg":{"timeout_ms":2000,"cache_bytes":33554432,"pdf_page_bytes":262144,"max_bytes":262144,"max_lines":400,"max_image_dimension":4096,"dir_budget":20000}});
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let line = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(line) => line,
            Err(e) => {
                let _ = child.kill();
                panic!("parser did not reply: {e}");
            }
        };
        let reply: Value = serde_json::from_str(&line).unwrap();
        let doc = &reply["Ok"];
        assert_eq!(doc["kind"], "PDF", "{reply}");
        let pages = doc["content"]["Pages"].as_array().unwrap();
        assert_eq!(pages.len(), count);
        assert_eq!(pages[0]["number"], page);
        assert_eq!(doc["next_page"], next);
        assert!(pages[0]["text"]
            .as_str()
            .unwrap()
            .contains(&format!("Page {page} text")));
        if page == 1 {
            std::fs::remove_file(&path).unwrap();
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    assert!(!config.exists());
}

#[test]
fn archive_worker_creates_an_interoperable_zip_and_extracts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("note.txt");
    std::fs::write(&source, b"A note\n").unwrap();
    let archive = tmp.path().join("result.zip");
    let run = |request: Value| {
        let path = tmp.path().join("request.json");
        std::fs::write(&path, request.to_string()).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_starfold"))
            .arg("--archive-worker")
            .arg(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let last = String::from_utf8(out.stdout).unwrap();
        let reply: Value = serde_json::from_str(last.lines().last().unwrap()).unwrap();
        assert!(reply["Done"].get("Ok").is_some(), "{reply}");
    };
    run(
        json!({"Create":{"format":"Zip","output":archive,"items":[{"from":source,"name":"note.txt","bytes":7}]}}),
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(zip.by_index(0).unwrap().name(), "note.txt");
    let dest = tmp.path().join("extracted");
    run(json!({"Extract":{"source":archive,"output":dest}}));
    assert_eq!(std::fs::read(dest.join("note.txt")).unwrap(), b"A note\n");
}
