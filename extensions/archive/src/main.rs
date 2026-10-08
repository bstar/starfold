fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("starfold-archive {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    starfold_archive::serve_stdio()
}
