mod args;
mod commands;
mod paths;
mod pictures;

use clap::Parser as _;
use std::io::Write as _;
use std::process::ExitCode;

/// Exit code for usage errors; 2 is reserved for `--strict` quality failures.
pub const EXIT_USAGE: u8 = 64;

fn main() -> ExitCode {
    let cli = match args::Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { EXIT_USAGE } else { 0 });
        }
    };
    if !cli.verbose {
        // Panics are reported as `ParseError::Internal`; keep stderr to one line.
        std::panic::set_hook(Box::new(|_| {
            let _ = writeln!(
                std::io::stderr(),
                "error: internal panic (use --verbose for details)"
            );
        }));
    }
    ExitCode::from(commands::run(cli))
}
