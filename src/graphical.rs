//! Experimental launcher and persistent application host. No browser runs remotely.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use starkit::terminal_graphics::{
    client::{self, Launch},
    session,
};

#[derive(Parser)]
#[command(
    name = "starfold-graphical",
    about = "STAR/FOLD graphical interface inside Kitty"
)]
struct Options {
    #[arg(long)]
    ssh: Option<String>,
    #[arg(long)]
    ssh_config: Option<PathBuf>,
    #[arg(long, default_value = "default")]
    session: String,
    #[arg(long)]
    attach: bool,
    #[arg(long)]
    sessions: bool,
    /// Report local transport, geometry and input capabilities without attaching.
    #[arg(long)]
    capabilities: bool,
    #[arg(long, default_value = "starfold")]
    remote_executable: String,
    directory: Option<PathBuf>,
}
#[derive(Parser)]
struct Relay {
    #[arg(long)]
    graphical_relay: Option<String>,
    #[arg(long)]
    graphical_server: Option<String>,
    #[arg(long)]
    directory: Option<PathBuf>,
    #[arg(long)]
    attach_only: bool,
    #[arg(long)]
    graphical_sessions: bool,
}

pub fn handles_args() -> bool {
    let first = std::env::args().nth(1).unwrap_or_default();
    first == "graphical" || first.starts_with("--graphical-")
}
fn root() -> Result<PathBuf> {
    // HOME is resolved by the host, so local and SSH sessions remain independent.
    Ok(crate::PATHS.base_dir()?.join("graphical"))
}
fn ensure(name: &str, dir: Option<PathBuf>, attach_only: bool) -> Result<PathBuf> {
    let root = root()?;
    session::private_root(&root)?;
    let socket = session::socket_path(&root, name)?;
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        return Ok(socket);
    }
    if attach_only {
        bail!("Session '{name}' is no longer running. Start a new session explicitly.");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(format!("{name}.log")))?;
    let mut command = Command::new(std::env::current_exe()?);
    command.args(["--graphical-server", name]);
    if let Some(dir) = dir {
        command.arg("--directory").arg(dir);
    }
    use std::os::unix::process::CommandExt;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            return Ok(socket);
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "Graphical session failed to start ({status}); see {}",
                root.join(format!("{name}.log")).display()
            );
        }
        if Instant::now() >= deadline {
            bail!("Timed out waiting for graphical session");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("graphical") {
        let args =
            std::iter::once("starfold-graphical".to_string()).chain(std::env::args().skip(2));
        let options = Options::parse_from(args);
        if options.capabilities {
            let graphics = starkit::graphics::Graphics::probe_if_tty(starkit::graphics::Mode::Auto);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &starkit::terminal_graphics::capabilities::Capabilities::detected(&graphics)
                )?
            );
            return Ok(());
        }
        session::socket_path(&root()?, &options.session)?;
        if options.sessions {
            if let Some(host) = options.ssh {
                if host.starts_with('-') {
                    bail!("Invalid SSH host");
                }
                let remote = format!(
                    "{} --graphical-sessions",
                    client::shell_quote(&options.remote_executable)
                );
                let mut ssh = Command::new("ssh");
                if let Some(config) = options.ssh_config {
                    ssh.arg("-F").arg(config);
                }
                let status = ssh.args(["-T", "--", &host, &remote]).status()?;
                if !status.success() {
                    bail!("Remote session listing failed");
                }
            } else {
                for name in session::list(&root()?)? {
                    println!("{name}");
                }
            }
            return Ok(());
        }
        crate::PATHS.init_private_dirs();
        let _log = starkit::logging::init(&crate::PATHS, true)?;
        return client::run_with_events(
            Launch {
                executable: if options.ssh.is_some() {
                    options.remote_executable
                } else {
                    std::env::current_exe()?.to_string_lossy().into_owned()
                },
                host: options.ssh,
                ssh_config: options.ssh_config,
                session: options.session,
                directory: options.directory.map(|p| p.to_string_lossy().into_owned()),
                attach_only: options.attach,
            },
            |event| match event {
                // A release may happen in another window. Never retain a
                // captured drag/scrollbar when the terminal loses focus.
                starkit::crossterm::event::Event::FocusLost => {
                    Some(starkit::terminal_graphics::Input::CancelPointer)
                }
                starkit::crossterm::event::Event::Osc72(text) => {
                    Some(starkit::terminal_graphics::Input::Osc72 { text: text.clone() })
                }
                _ => None,
            },
        );
    }
    let options = Relay::parse();
    if options.graphical_sessions {
        for name in session::list(&root()?)? {
            println!("{name}");
        }
        return Ok(());
    }
    if let Some(name) = options.graphical_relay {
        let socket = ensure(&name, options.directory, options.attach_only)?;
        return session::relay(&socket);
    }
    let name = options
        .graphical_server
        .context("Missing graphical session name")?;
    let root = root()?;
    session::private_root(&root)?;
    session::socket_path(&root, &name)?;
    crate::PATHS.init_private_dirs();
    let _log = starkit::logging::init(&crate::PATHS, true)?;
    let saved = root.join(format!("{name}.toml"));
    if !saved.exists() {
        let normal = crate::PATHS.session_file()?;
        if normal.exists() {
            std::fs::copy(normal, &saved)?;
        }
    }
    let (core, cfg, path, saved) = crate::window_parts(options.directory, Some(saved))?;
    let mut app = crate::ui::app::App::new(
        core,
        cfg,
        path,
        Some(saved),
        starkit::graphics::Graphics::disabled(),
    );
    app.enable_graphical();
    session::serve(&root, &name, app)
}
