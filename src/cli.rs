//! The command line.
//!
//! `starfold` with no arguments opens the window on the session's last
//! directory, or the current one on a first run. `starfold list` is a
//! headless listing -- STAR/CORD's `probe` equivalent -- and it exists for
//! the same reason: the core is written before there is a terminal to drive
//! it, and printing what a directory holds with no TTY attached stays useful
//! afterwards, for a script or a bug report.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::fold::sort::SortKey;

#[derive(Debug, Parser)]
#[command(
    name = "starfold",
    version,
    about = "STAR/FOLD — a stack-based terminal file manager",
    long_about = None,
)]
pub struct Cli {
    /// Log at debug level. To the log file, never to the terminal.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Directory to open (default: the session's last directory, else the
    /// current directory).
    pub dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List a directory and exit, with no terminal involved.
    List {
        /// The directory to list.
        dir: PathBuf,

        /// Include entries whose name starts with `.`.
        #[arg(long)]
        hidden: bool,

        /// How to order the listing.
        #[arg(long, value_enum, default_value_t)]
        sort: SortArg,
    },
}

/// [`SortKey`] as a command-line value. A separate type because `clap`'s
/// derive wants `ValueEnum` on the type it parses into, and `fold::sort` --
/// under `src/fold/`, where nothing may know a terminal exists -- has no
/// business depending on `clap` to get it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum SortArg {
    #[default]
    Name,
    Size,
    Time,
    Ext,
}

impl std::fmt::Display for SortArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}

impl SortArg {
    pub fn key(self) -> SortKey {
        match self {
            SortArg::Name => SortKey::Name,
            SortArg::Size => SortKey::Size,
            SortArg::Time => SortKey::Time,
            SortArg::Ext => SortKey::Ext,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_definition_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_opens_the_window_on_the_default_directory() {
        let cli = Cli::parse_from(["starfold"]);
        assert!(cli.command.is_none());
        assert!(cli.dir.is_none());
        assert!(!cli.verbose);
    }

    #[test]
    fn a_bare_directory_argument_is_where_to_open() {
        let cli = Cli::parse_from(["starfold", "/tmp"]);
        assert_eq!(cli.dir.as_deref(), Some(std::path::Path::new("/tmp")));
        assert!(cli.command.is_none());
    }

    #[test]
    fn list_takes_a_directory_and_defaults_to_a_visible_name_sort() {
        let cli = Cli::parse_from(["starfold", "list", "/tmp"]);
        match cli.command {
            Some(Command::List { dir, hidden, sort }) => {
                assert_eq!(dir, std::path::PathBuf::from("/tmp"));
                assert!(!hidden);
                assert_eq!(sort, SortArg::Name);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn list_accepts_hidden_and_a_chosen_sort() {
        let cli = Cli::parse_from(["starfold", "list", "/tmp", "--hidden", "--sort", "size"]);
        match cli.command {
            Some(Command::List { hidden, sort, .. }) => {
                assert!(hidden);
                assert_eq!(sort, SortArg::Size);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_sort_arg_maps_to_a_sort_key() {
        assert_eq!(SortArg::Name.key(), SortKey::Name);
        assert_eq!(SortArg::Size.key(), SortKey::Size);
        assert_eq!(SortArg::Time.key(), SortKey::Time);
        assert_eq!(SortArg::Ext.key(), SortKey::Ext);
    }

    #[test]
    fn verbose_is_accepted_before_and_after_the_subcommand() {
        assert!(Cli::parse_from(["starfold", "--verbose", "list", "/tmp"]).verbose);
        assert!(Cli::parse_from(["starfold", "list", "/tmp", "--verbose"]).verbose);
    }
}
