//! The repository's own tooling, run as `cargo xtask <task>` from the repository root (the
//! alias is in `.cargo/config.toml`).
//!
//! xtask/ is a workspace of its own, so neither the Docker build nor the crates.io package
//! sees it. CI runs its tests separately (the `test` job in .github/workflows/ci.yml).

mod changelog;
mod date;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask changelog check
       cargo xtask changelog preview
       cargo xtask changelog release <version> [--date YYYY-MM-DD] [--dry-run]

  check    validate the entries under changelog.d/
  preview  print the section they would make, under `## Unreleased`
  release  gather them, together with any hand-written `## Unreleased` entries, into a
           `## <version> - <date>` section of CHANGELOG.md and delete them. The date
           defaults to today; --dry-run prints the section and changes nothing.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ is inside the repository");
    match run(root, &args) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, args: &[String]) -> Result<String, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["help" | "--help" | "-h"] => Ok(USAGE.to_string()),
        ["changelog", "check"] => changelog::check(root),
        ["changelog", "preview"] => changelog::preview(root),
        ["changelog", "release", rest @ ..] => {
            let mut version = None;
            let mut date = None;
            let mut dry_run = false;
            let mut rest = rest.iter();
            while let Some(&arg) = rest.next() {
                match arg {
                    "--dry-run" => dry_run = true,
                    "--date" => date = Some(*rest.next().ok_or("--date needs a value")?),
                    _ if arg.starts_with("--date=") => date = Some(&arg["--date=".len()..]),
                    _ if arg.starts_with('-') => {
                        return Err(format!("unknown option {arg}\n\n{USAGE}"))
                    }
                    _ if version.is_none() => version = Some(arg),
                    _ => return Err(format!("unexpected argument {arg}\n\n{USAGE}")),
                }
            }
            let version = version.ok_or_else(|| format!("release needs a version\n\n{USAGE}"))?;
            let date = match date {
                Some(date) => date.to_string(),
                None => date::today(),
            };
            changelog::release(root, version, &date, dry_run)
        }
        _ => Err(USAGE.to_string()),
    }
}
