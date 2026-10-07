//! Experimental launcher and persistent application host. No browser runs remotely.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use starkit::terminal_graphics::{
    client::{self, Launch, PresentationOptions},
    session,
};

#[derive(Parser)]
#[command(
    name = "starfold-graphical",
    version,
    about = "STAR/FOLD — graphical and cell interfaces"
)]
struct Options {
    #[arg(long)]
    ssh: Option<String>,
    /// Start in cell mode for this launch; F9 still switches live.
    #[arg(long, conflicts_with = "graphical")]
    cells: bool,
    /// Start graphically for this launch where supported.
    #[arg(long)]
    graphical: bool,
    #[arg(short, long)]
    verbose: bool,
    #[arg(long)]
    ssh_config: Option<PathBuf>,
    /// Session name. Defaults to local on this machine, default over SSH.
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    attach: bool,
    #[arg(long)]
    sessions: bool,
    /// Report local transport, geometry and input capabilities without attaching.
    #[arg(long)]
    capabilities: bool,
    #[arg(long, default_value = "starfold")]
    remote_executable: String,
    /// Open a movie directly in desktop full screen, on the selected host.
    #[arg(long)]
    play: Option<String>,
    directory: Option<PathBuf>,
}
impl Options {
    fn session_name(&self, ssh_shell: bool) -> &str {
        self.session
            .as_deref()
            .unwrap_or(if self.ssh.is_some() || ssh_shell {
                "default"
            } else {
                "local"
            })
    }
}
#[derive(Parser)]
struct Relay {
    #[arg(long)]
    graphical_terminal_client: Option<PathBuf>,
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

fn terminal_event(
    event: &starkit::crossterm::event::Event,
) -> Option<starkit::terminal_graphics::Input> {
    match event {
        starkit::crossterm::event::Event::FocusLost => {
            Some(starkit::terminal_graphics::Input::CancelPointer)
        }
        starkit::crossterm::event::Event::Osc72(text) => {
            Some(starkit::terminal_graphics::Input::Osc72 { text: text.clone() })
        }
        _ => None,
    }
}

pub fn handles_args() -> bool {
    let first = std::env::args().nth(1).unwrap_or_default();
    first == "graphical"
        || first.starts_with("--graphical-")
        || matches!(
            first.as_str(),
            "--cells"
                | "--graphical"
                | "--ssh"
                | "--ssh-config"
                | "--session"
                | "--attach"
                | "--sessions"
                | "--capabilities"
                | "--play"
                | "--remote-executable"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        )
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
    // A persistent AppImage host needs its own runtime/mount. Launching the
    // current payload would leave its helpers/workers under the client's
    // disposable APPDIR after that client exits.
    let appimage = if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE")
    } else {
        None
    };
    let mut command = Command::new(
        appimage
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or(std::env::current_exe()?),
    );
    if appimage.is_some() {
        command.env_remove("APPIMAGE").env_remove("APPDIR");
    }
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
fn presentation_options() -> Result<PresentationOptions> {
    let config = crate::config::Config::load(&crate::PATHS.config_file()?)?;
    Ok(PresentationOptions {
        pane_corner_radius: config.ui.pane_radius(),
        video_corner_radius: config.preview.video_radius(),
    })
}

pub fn main() -> Result<()> {
    if !std::env::args()
        .nth(1)
        .unwrap_or_default()
        .starts_with("--graphical-")
    {
        let skip = if std::env::args().nth(1).as_deref() == Some("graphical") {
            2
        } else {
            1
        };
        let args = std::iter::once("starfold".to_string()).chain(std::env::args().skip(skip));
        let options = Options::parse_from(args);
        let play = options.play.as_ref().map(|path| {
            if options.ssh.is_none()
                && !path.starts_with("~/")
                && !PathBuf::from(path).is_absolute()
            {
                std::env::current_dir()
                    .map(|dir| dir.join(path).to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.clone())
            } else {
                path.clone()
            }
        });
        let session = options
            .session_name(std::env::var_os("SSH_CONNECTION").is_some())
            .to_owned();
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
        session::socket_path(&root()?, &session)?;
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
        // A registered Kitty frontend can stay local while the user launches
        // normally inside an existing SSH shell. Reuse that SSH TTY; no second
        // authentication, inferred hostname, or new connection is needed.
        if options.ssh.is_none() {
            if let Some(terminal) = starkit::terminal_graphics::terminal_bridge::probe()? {
                let socket = ensure(&session, options.directory, options.attach)?;
                return terminal.relay_with_play(&socket, play);
            }
        }
        crate::PATHS.init_private_dirs();
        let _log = starkit::logging::init(&crate::PATHS, true)?;
        let config_path = crate::PATHS.config_file()?;
        let config = crate::config::Config::load(&config_path)?;
        return client::run_with_preference(
            Launch {
                executable: if options.ssh.is_some() {
                    options.remote_executable
                } else {
                    std::env::current_exe()?.to_string_lossy().into_owned()
                },
                host: options.ssh,
                ssh_config: options.ssh_config,
                session,
                directory: options.directory.map(|p| p.to_string_lossy().into_owned()),
                attach_only: options.attach,
                play,
            },
            terminal_event,
            crate::updates::frontend_notice,
            presentation_options()?,
            client::PresentationPreference {
                cells: options.cells
                    || (!options.graphical
                        && config.ui.presentation == crate::config::PresentationMode::Cells),
                path: config_path,
            },
        );
    }
    let options = Relay::parse();
    if let Some(socket) = options.graphical_terminal_client {
        crate::PATHS.init_private_dirs();
        let _log = starkit::logging::init(&crate::PATHS, true)?;
        let path = crate::PATHS.config_file()?;
        let config = crate::config::Config::load(&path)?;
        return client::run_terminal_socket_with_preference(
            &socket,
            terminal_event,
            presentation_options()?,
            client::PresentationPreference {
                cells: config.ui.presentation == crate::config::PresentationMode::Cells,
                path,
            },
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_and_remote_default_sessions_do_not_collide() {
        let local = Options::parse_from(["starfold-graphical"]);
        assert_eq!(local.session_name(false), "local");
        assert_eq!(local.session_name(true), "default");
        let remote = Options::parse_from(["starfold-graphical", "--ssh", "host"]);
        assert_eq!(remote.session_name(false), "default");
        let shared = Options::parse_from(["starfold-graphical", "--session", "default"]);
        assert_eq!(shared.session_name(false), "default");
        let named = Options::parse_from(["starfold-graphical", "--session", "work", "--attach"]);
        assert_eq!(named.session_name(false), "work");
        assert_eq!(named.session_name(true), "work");
    }
}
