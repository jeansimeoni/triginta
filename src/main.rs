// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

use anyhow::Result;
use clap::{Arg, ArgAction, Command, error::ErrorKind};

#[derive(Debug)]
enum CliAction {
    Run(triginta::app::RunOptions),
    PrintAndExit(String),
}

fn main() -> Result<()> {
    // In C this would usually be `int main(void)` plus explicit error codes.
    // Rust programs commonly return `Result` from `main`, which lets the `?`
    // operator propagate failures instead of manually checking every call.
    match parse_cli_action(std::env::args_os()) {
        Ok(CliAction::Run(options)) => triginta::app::run(options),
        Ok(CliAction::PrintAndExit(output)) => {
            print!("{output}");
            Ok(())
        }
        Err(error) => error.exit(),
    }
}

fn parse_cli_action<I, T>(args: I) -> std::result::Result<CliAction, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    parse_cli_action_with_debug_flags(args, cfg!(debug_assertions))
}

fn parse_cli_action_with_debug_flags<I, T>(
    args: I,
    include_debug_flags: bool,
) -> std::result::Result<CliAction, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    match cli_command(include_debug_flags).try_get_matches_from(args) {
        Ok(matches) => {
            if !include_debug_flags {
                return Ok(CliAction::Run(triginta::app::RunOptions::default()));
            }

            Ok(CliAction::Run(triginta::app::RunOptions {
                force_ascii: matches.get_flag("ascii"),
                force_short_timer: matches.get_flag("short-timer"),
                reset_data: matches.get_flag("reset-data"),
                dry_run_sync: matches.get_flag("dry-run-sync"),
                local_only: matches.get_flag("local-only"),
            }))
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            Ok(CliAction::PrintAndExit(error.to_string()))
        }
        Err(error) => Err(error),
    }
}

fn cli_command(include_debug_flags: bool) -> Command {
    let mut command = Command::new("triginta")
        .about("A local-first TUI Pomodoro timer and task manager.")
        .version(env!("CARGO_PKG_VERSION"))
        .arg_required_else_help(false)
        .disable_help_subcommand(true);

    if include_debug_flags {
        command = command
            .arg(hidden_debug_flag("ascii"))
            .arg(hidden_debug_flag("short-timer"))
            .arg(hidden_debug_flag("reset-data"))
            .arg(hidden_debug_flag("dry-run-sync"))
            .arg(hidden_debug_flag("local-only"));
    }

    command
}

fn hidden_debug_flag(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .action(ArgAction::SetTrue)
        .hide(true)
}

#[cfg(test)]
mod tests {
    use super::{CliAction, parse_cli_action_with_debug_flags};
    use clap::error::ErrorKind;

    #[test]
    fn help_prints_without_run_options() {
        let action = parse_cli_action_with_debug_flags(["triginta", "--help"], true)
            .expect("help should parse as an exit action");

        let CliAction::PrintAndExit(output) = action else {
            panic!("help should not start the app");
        };

        assert!(output.contains("Usage: triginta"));
        assert!(output.contains("--version"));
        assert!(!output.contains("--ascii"));
    }

    #[test]
    fn version_prints_without_run_options() {
        let action = parse_cli_action_with_debug_flags(["triginta", "--version"], true)
            .expect("version should parse as an exit action");

        let CliAction::PrintAndExit(output) = action else {
            panic!("version should not start the app");
        };

        assert!(output.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn unknown_flag_returns_cli_error() {
        let error = parse_cli_action_with_debug_flags(["triginta", "--unknown"], true)
            .expect_err("unknown flags should fail");

        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn debug_flags_populate_run_options_when_enabled() {
        let action = parse_cli_action_with_debug_flags(
            [
                "triginta",
                "--ascii",
                "--short-timer",
                "--reset-data",
                "--dry-run-sync",
                "--local-only",
            ],
            true,
        )
        .expect("debug flags should parse in debug-style mode");

        let CliAction::Run(options) = action else {
            panic!("debug flags should run the app");
        };

        assert!(options.force_ascii);
        assert!(options.force_short_timer);
        assert!(options.reset_data);
        assert!(options.dry_run_sync);
        assert!(options.local_only);
    }

    #[test]
    fn no_args_defaults_when_debug_flags_are_disabled() {
        let action = parse_cli_action_with_debug_flags(["triginta"], false)
            .expect("no-arg release-style parse should start the app");

        let CliAction::Run(options) = action else {
            panic!("no-arg release-style parse should run the app");
        };

        assert!(!options.force_ascii);
        assert!(!options.force_short_timer);
        assert!(!options.reset_data);
        assert!(!options.dry_run_sync);
        assert!(!options.local_only);
    }

    #[test]
    fn debug_flags_are_rejected_when_disabled() {
        for flag in [
            "--ascii",
            "--short-timer",
            "--reset-data",
            "--dry-run-sync",
            "--local-only",
        ] {
            let error = parse_cli_action_with_debug_flags(["triginta", flag], false)
                .expect_err("debug flags should fail in release-style mode");

            assert_eq!(error.kind(), ErrorKind::UnknownArgument);
        }
    }
}
