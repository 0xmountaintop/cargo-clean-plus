use anyhow::{bail, Result};
use clap::Parser;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use std::{
    env::current_dir,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};
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

struct CleanupStats {
    projects: usize,
    files: usize,
    size_kib: f64,
}

impl CleanupStats {
    fn new() -> Self {
        Self {
            projects: 0,
            files: 0,
            size_kib: 0.0,
        }
    }

    fn format_size(&self) -> String {
        if self.size_kib > 1024.0 * 1024.0 {
            format!("{:.2}GiB", self.size_kib / 1024.0 / 1024.0)
        } else if self.size_kib > 1024.0 {
            format!("{:.2}MiB", self.size_kib / 1024.0)
        } else {
            format!("{:.2}KiB", self.size_kib)
        }
    }
}

struct TimeParser;

impl TimeParser {
    fn parse_duration(time_str: &str) -> Result<Duration> {
        let (value, unit) = time_str.split_at(time_str.len() - 1);
        let value: u64 = value.parse()?;
        
        let seconds = match unit {
            "m" => value * 60,
            "h" => value * 60 * 60,
            "d" => value * 60 * 60 * 24,
            "w" => value * 60 * 60 * 24 * 7,
            _ => bail!("Unknown unit, available units: m, h, d, w"),
        };
        
        Ok(Duration::from_secs(seconds))
    }
}

struct CargoProject {
    path: PathBuf,
    regex: Regex,
}

impl CargoProject {
    fn new(path: PathBuf) -> Self {
        let regex = Regex::new(
            r"Removed (?P<files>\d+) files, (?P<size>\d+(?:\.\d+)?)(?P<unit>\w+) total",
        ).expect("Invalid regex pattern");
        
        Self { path, regex }
    }

    fn is_valid_project(&self) -> bool {
        self.path.join("Cargo.toml").exists() && self.path.join("target").exists()
    }

    fn clean(&self) -> Result<Option<(usize, f64)>> {
        let output = Command::new("cargo")
            .arg("clean")
            .current_dir(&self.path)
            .output()?;

        if !output.status.success() {
            return Ok(None);
        }

        let output = String::from_utf8_lossy(&output.stderr);
        if output.contains("Removed 0 files") {
            return Ok(None);
        }

        let caps = self.regex.captures(&output)
            .expect("Failed to parse cargo clean output");
        
        let files = caps["files"].parse::<usize>()?;
        let size = caps["size"].parse::<f64>()?;
        let size_kib = match &caps["unit"] {
            "KiB" => size,
            "MiB" => size * 1024.0,
            "GiB" => size * 1024.0 * 1024.0,
            _ => unreachable!("Unknown unit"),
        };

        Ok(Some((files, size_kib)))
    }
}

fn setup_progress_bar() -> Result<ProgressBar> {
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{prefix:>12.bold.green} {msg}")?);
    Ok(pb)
}

fn process_directory(dir: &Path, before: SystemTime) -> Result<CleanupStats> {
    let pb = setup_progress_bar()?;
    let mut stats = CleanupStats::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        pb.set_prefix("Scanning");
        pb.set_message(format!("{}", entry.path().display()));

        let project = CargoProject::new(entry.path().to_path_buf());
        if !project.is_valid_project() {
            continue;
        }

        if let Some(modified) = entry.metadata()?.modified()? {
            if modified > before {
                continue;
            }
        }

        if let Ok(Some((files, size))) = project.clean() {
            stats.projects += 1;
            stats.files += files;
            stats.size_kib += size;

            pb.println(format!(
                "{:>12} {} files, {:.2}{} total in {}",
                style("Removed").bold().green(),
                files,
                size,
                if size >= 1024.0 * 1024.0 { "GiB" } else if size >= 1024.0 { "MiB" } else { "KiB" },
                entry.path().display()
            ));
        }
    }

    pb.set_prefix("Cleaned");
    pb.finish_with_message(format!(
        "{} projects, {} files, {} total",
        stats.projects,
        stats.files,
        stats.format_size(),
    ));

    Ok(stats)
}

fn main() -> Result<()> {
    let cmd = Cli::parse();
    let dir = cmd.dir.unwrap_or_else(|| current_dir().expect("Failed to get current directory"));
    
    let past = cmd.past.as_deref().unwrap_or("0m");
    let duration = TimeParser::parse_duration(past)?;
    
    let now = SystemTime::now();
    let before = now.checked_sub(duration).unwrap_or(now);

    process_directory(&dir, before)?;
    Ok(())
}
