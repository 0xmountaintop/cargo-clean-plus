use clap::Parser;
use console::style;
use std::env::current_dir;
use std::path::PathBuf;
use std::process::Command;
use anyhow::bail;
use indicatif::ProgressStyle;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Base directory to run cleanup scan
    dir: Option<PathBuf>,
    /// Only clean project that hasn't been touched for a certain period, available units: m, h, d, w
    #[clap(short, long)]
    past: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cmd = Cli::parse();
    let dir = cmd
        .dir
        .unwrap_or_else(|| current_dir().expect("Failed to get current directory"));

    let past = cmd.past.as_deref().unwrap_or("0m");
    let past = if past.ends_with('m') {
        past.trim_end_matches('m').parse::<u64>()? * 60
    } else if past.ends_with('h') {
        past.trim_end_matches('h').parse::<u64>()? * 60 * 60
    } else if past.ends_with('d') {
        past.trim_end_matches('d').parse::<u64>()? * 60 * 60 * 24
    } else if past.ends_with('w') {
        past.trim_end_matches('w').parse::<u64>()? * 60 * 60 * 24 * 7
    } else {
        bail!("Unknown unit, available units: m, h, d, w");
    };
    let now = std::time::SystemTime::now();
    let before = now.checked_sub(std::time::Duration::from_secs(past)).unwrap_or(now);

    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{prefix:>12.bold.green} {msg}")?);

    let mut cleaned_projects = 0usize;
    let mut removed_files = 0usize;
    let mut removed_size = 0.;
    // cargo clean output format is: \s+ Removed \d+ files, \d+(.\d+)? KiB/MiB/GiB total
    let re = regex::Regex::new(
        r"Removed (?P<files>\d+) files, (?P<size>\d+(?:\.\d+)?)(?P<unit>\w+) total",
    )?;

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        pb.set_prefix("Scanning");
        pb.set_message(format!("{}", entry.path().display()));

        // skip if not a cargo project
        if !entry.path().join("Cargo.toml").exists() {
            continue;
        }
        // skip if /target not exists or is empty
        let target_dir = entry.path().join("target");
        if !target_dir.exists() {
            continue;
        }
        // skip if modified after the specified time
        if let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
            if modified > before {
                continue;
            }
        }

        // spawn cargo clean and eats the output
        if let Ok(out) = Command::new("cargo")
            .arg("clean")
            .current_dir(entry.path())
            .output()
        {
            if !out.status.success() {
                continue;
            }

            let out = String::from_utf8_lossy(&out.stderr);
            if out.contains("Removed 0 files") {
                continue;
            }

            cleaned_projects += 1;
            let caps = re.captures(&out).expect("Failed to parse cargo clean output");
            let files = caps["files"].parse::<usize>()?;
            let size = caps["size"].parse::<f64>()?;
            let unit = &caps["unit"];
            removed_size += match unit {
                "KiB" => size,
                "MiB" => size * 1024.,
                "GiB" => size * 1024. * 1024.,
                _ => unreachable!("Unknown unit"),
            };
            removed_files += files;

            pb.println(format!(
                "{:>12} {} files, {}{} total in {}",
                style("Removed").bold().green(),
                files,
                size,
                unit,
                entry.path().display()
            ));
        }
    }

    let removed_size = if removed_size > 1024. {
        format!("{:.2}MiB", removed_size / 1024.)
    } else if removed_size > 1024. * 1024. {
        format!("{:.2}GiB", removed_size / 1024. / 1024.)
    } else {
        format!("{:.2}KiB", removed_size)
    };

    pb.set_prefix("Cleaned");
    pb.finish_with_message(format!(
        "{cleaned_projects} projects, {removed_files} files, {removed_size} total",
    ));

    Ok(())
}
