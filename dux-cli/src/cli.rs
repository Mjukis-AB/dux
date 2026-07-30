use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// DUX - Interactive Terminal Disk Usage Analyzer
#[derive(Parser, Debug)]
#[command(name = "dux")]
#[command(about = "Understand disk usage and recover space safely")]
#[command(version)]
#[command(args_conflicts_with_subcommands = true)]
#[command(subcommand_precedence_over_arg = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    #[command(flatten)]
    pub(crate) tui: TuiArgs,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    #[command(name = "__bundle-metadata", hide = true)]
    BundleMetadata,
    /// Show durable engine and latest-scan status
    Status(OutputArgs),
    /// Show recent durable scan history
    History(HistoryArgs),
}

#[derive(Args, Clone, Debug)]
pub(crate) struct TuiArgs {
    /// Path to analyze (defaults to current directory)
    pub(crate) path: Option<PathBuf>,

    /// Maximum depth to scan
    #[arg(short, long)]
    pub(crate) max_depth: Option<usize>,

    /// Follow symbolic links
    #[arg(short, long)]
    pub(crate) follow_symlinks: bool,

    /// Cross filesystem boundaries
    #[arg(short = 'x', long)]
    pub(crate) cross_filesystems: bool,

    /// Disable cache (always perform fresh scan)
    #[arg(long)]
    pub(crate) no_cache: bool,
}

impl TuiArgs {
    pub(crate) fn path(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| PathBuf::from("."))
    }
}

#[derive(Args, Clone, Copy, Debug)]
pub(crate) struct OutputArgs {
    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Clone, Copy, Debug)]
pub(crate) struct HistoryArgs {
    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,

    /// Maximum number of recent scans to return
    #[arg(long, default_value_t = 20, value_parser = parse_history_limit)]
    pub(crate) limit: usize,
}

fn parse_history_limit(value: &str) -> Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| "history limit must be an integer between 1 and 200".to_owned())?;
    if !(1..=200).contains(&limit) {
        return Err("history limit must be between 1 and 200".to_owned());
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    #[test]
    fn bare_invocation_preserves_tui_defaults() {
        let cli = Cli::try_parse_from(["dux"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.tui.path(), PathBuf::from("."));
        assert_eq!(cli.tui.max_depth, None);
        assert!(!cli.tui.follow_symlinks);
        assert!(!cli.tui.cross_filesystems);
        assert!(!cli.tui.no_cache);
    }

    #[test]
    fn existing_tui_path_and_flags_remain_accepted() {
        let cli = Cli::try_parse_from([
            "dux",
            "--max-depth",
            "3",
            "--follow-symlinks",
            "--cross-filesystems",
            "--no-cache",
            "/tmp",
        ])
        .unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.tui.path(), PathBuf::from("/tmp"));
        assert_eq!(cli.tui.max_depth, Some(3));
        assert!(cli.tui.follow_symlinks);
        assert!(cli.tui.cross_filesystems);
        assert!(cli.tui.no_cache);
    }

    #[test]
    fn status_and_history_are_explicit_noninteractive_commands() {
        let status = Cli::try_parse_from(["dux", "status", "--json"]).unwrap();
        assert!(matches!(
            status.command,
            Some(Command::Status(OutputArgs { json: true }))
        ));

        let history = Cli::try_parse_from(["dux", "history", "--json", "--limit", "200"]).unwrap();
        assert!(matches!(
            history.command,
            Some(Command::History(HistoryArgs {
                json: true,
                limit: 200
            }))
        ));
    }

    #[test]
    fn bundle_metadata_is_an_exact_hidden_command_without_arguments() {
        let metadata = Cli::try_parse_from(["dux", "__bundle-metadata"]).unwrap();
        assert!(matches!(metadata.command, Some(Command::BundleMetadata)));

        let unexpected = Cli::try_parse_from(["dux", "__bundle-metadata", "--json"]).unwrap_err();
        assert_eq!(unexpected.kind(), ErrorKind::UnknownArgument);

        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("__bundle-metadata"));
    }

    #[test]
    fn reserved_command_names_still_work_as_explicit_paths() {
        let relative = Cli::try_parse_from(["dux", "./status"]).unwrap();
        assert!(relative.command.is_none());
        assert_eq!(relative.tui.path(), PathBuf::from("./status"));

        let after_delimiter = Cli::try_parse_from(["dux", "--", "history"]).unwrap();
        assert!(after_delimiter.command.is_none());
        assert_eq!(after_delimiter.tui.path(), PathBuf::from("history"));
    }

    #[test]
    fn subcommands_reject_tui_flags_and_out_of_range_limits() {
        let conflict = Cli::try_parse_from(["dux", "status", "--no-cache"]).unwrap_err();
        assert_eq!(conflict.kind(), ErrorKind::UnknownArgument);

        for invalid in ["0", "201"] {
            let error = Cli::try_parse_from(["dux", "history", "--limit", invalid]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ValueValidation);
        }
    }
}
