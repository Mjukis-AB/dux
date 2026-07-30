use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Show bounded coverage details for one durable scan
    ScanDetail(ScanDetailArgs),
    /// Show bounded, path-free candidates for one durable scan
    Candidates(CandidatesArgs),
    /// Apply one semantic candidate-review transition
    ReviewState(ReviewStateArgs),
    /// Inspect bounded, path-free cleanup history
    CleanupHistory(CleanupHistoryArgs),
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

#[derive(Args, Clone, Debug)]
pub(crate) struct ScanDetailArgs {
    /// Exact durable scan identifier
    #[arg(long)]
    pub(crate) scan_id: dux_core::ScanId,

    /// Zero-based coverage issue offset
    #[arg(long, default_value_t = 0)]
    pub(crate) offset: u16,

    /// Maximum number of coverage issues to return
    #[arg(long, default_value_t = 20, value_parser = parse_detail_limit)]
    pub(crate) limit: u16,

    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CandidatesArgs {
    /// Exact durable scan identifier
    #[arg(long)]
    pub(crate) scan_id: dux_core::ScanId,

    /// Zero-based immutable candidate ordinal
    #[arg(long, default_value_t = 0)]
    pub(crate) cursor: u16,

    /// Maximum number of candidates to return
    #[arg(long, default_value_t = 20, value_parser = parse_detail_limit)]
    pub(crate) limit: u16,

    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct ReviewStateArgs {
    /// Exact durable scan identifier
    #[arg(long)]
    pub(crate) scan_id: dux_core::ScanId,

    /// Exact durable candidate identifier
    #[arg(long)]
    pub(crate) candidate_id: dux_core::CandidateId,

    /// Semantic review transition
    #[arg(long)]
    pub(crate) command: ReviewCommandArg,

    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ReviewCommandArg {
    Select,
    ClearSelection,
    Dismiss,
    Restore,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CleanupHistoryArgs {
    #[command(subcommand)]
    pub(crate) command: CleanupHistoryCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CleanupHistoryCommand {
    /// List recent cleanup-session summaries
    List(CleanupHistoryListArgs),
    /// Show one exact cleanup-session observation
    Show(CleanupHistoryShowArgs),
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CleanupHistoryListArgs {
    /// Maximum number of cleanup sessions to return
    #[arg(long, default_value_t = 20, value_parser = parse_detail_limit)]
    pub(crate) limit: u16,

    /// Cursor start time copied from a preceding response
    #[arg(long, requires = "after_session_id", value_parser = parse_nonnegative_unix_ms)]
    pub(crate) after_started_at_unix_ms: Option<i64>,

    /// Cursor session ID copied from a preceding response
    #[arg(long, requires = "after_started_at_unix_ms", value_parser = parse_cleanup_session_id)]
    pub(crate) after_session_id: Option<String>,

    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CleanupHistoryShowArgs {
    /// Exact cleanup-session ID copied from cleanup-history list
    #[arg(long, value_parser = parse_cleanup_session_id)]
    pub(crate) session_id: String,

    /// Emit the stable versioned JSON representation
    #[arg(long)]
    pub(crate) json: bool,
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

fn parse_detail_limit(value: &str) -> Result<u16, String> {
    let limit = value
        .parse::<u16>()
        .map_err(|_| "limit must be an integer between 1 and 64".to_owned())?;
    if !(1..=64).contains(&limit) {
        return Err("limit must be between 1 and 64".to_owned());
    }
    Ok(limit)
}

fn parse_nonnegative_unix_ms(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| "cursor time must be a non-negative Unix millisecond value".to_owned())
}

fn parse_cleanup_session_id(value: &str) -> Result<String, String> {
    dux_core::engine::DurableCleanupSessionId::from_stable_str(value)
        .map(|_| value.to_owned())
        .ok_or_else(|| "cleanup session ID must be a bounded stable token".to_owned())
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
    fn advanced_inspection_commands_have_exact_bounded_grammar() {
        let detail = Cli::try_parse_from([
            "dux",
            "scan-detail",
            "--scan-id",
            "scan:one",
            "--offset",
            "3",
            "--limit",
            "64",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            detail.command,
            Some(Command::ScanDetail(ScanDetailArgs {
                offset: 3,
                limit: 64,
                json: true,
                ..
            }))
        ));

        let candidates = Cli::try_parse_from([
            "dux",
            "candidates",
            "--scan-id",
            "scan:one",
            "--cursor",
            "4",
        ])
        .unwrap();
        assert!(matches!(
            candidates.command,
            Some(Command::Candidates(CandidatesArgs {
                cursor: 4,
                limit: 20,
                json: false,
                ..
            }))
        ));

        let review = Cli::try_parse_from([
            "dux",
            "review-state",
            "--scan-id",
            "scan:one",
            "--candidate-id",
            "candidate:one",
            "--command",
            "clear-selection",
        ])
        .unwrap();
        assert!(matches!(
            review.command,
            Some(Command::ReviewState(ReviewStateArgs {
                command: ReviewCommandArg::ClearSelection,
                ..
            }))
        ));
    }

    #[test]
    fn cleanup_history_cursor_is_an_atomic_pair() {
        let list = Cli::try_parse_from([
            "dux",
            "cleanup-history",
            "list",
            "--limit",
            "64",
            "--after-started-at-unix-ms",
            "1750000000000",
            "--after-session-id",
            "cleanup:one",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            list.command,
            Some(Command::CleanupHistory(CleanupHistoryArgs {
                command: CleanupHistoryCommand::List(CleanupHistoryListArgs {
                    limit: 64,
                    after_started_at_unix_ms: Some(1_750_000_000_000),
                    json: true,
                    ..
                })
            }))
        ));

        for args in [
            vec![
                "dux",
                "cleanup-history",
                "list",
                "--after-started-at-unix-ms",
                "1",
            ],
            vec![
                "dux",
                "cleanup-history",
                "list",
                "--after-session-id",
                "cleanup:one",
            ],
        ] {
            assert_eq!(
                Cli::try_parse_from(args).unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }
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
        for command in ["scan-detail", "candidates"] {
            for invalid in ["0", "65"] {
                let error = Cli::try_parse_from([
                    "dux",
                    command,
                    "--scan-id",
                    "scan:one",
                    "--limit",
                    invalid,
                ])
                .unwrap_err();
                assert_eq!(error.kind(), ErrorKind::ValueValidation);
            }
        }
    }
}
