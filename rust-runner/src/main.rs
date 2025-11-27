mod bpf_map;
mod stats;

use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};

use crate::bpf_map::{iterate_task_info, open_pinned_map};
use crate::stats::TaskInfo;

#[derive(Parser)]
#[command(author, version, about = "Observe sched_switch activity and derive lottery stats", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Dump the current BPF map contents at a fixed cadence
    Dump(DumpArgs),
}

#[derive(Args, Clone)]
struct DumpArgs {
    /// Path to the pinned task map
    #[arg(long, default_value = "/sys/fs/bpf/task_map")]
    map: String,

    /// Seconds to sleep between samples
    #[arg(long, default_value_t = 1)]
    interval: u64,

    /// Number of samples to capture
    #[arg(long, default_value_t = 10)]
    iterations: u32,

    /// Optional CSV file to append results to
    #[arg(long)]
    output: Option<PathBuf>,
}

fn main() {
    if let Err(err) = entry() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn entry() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dump(args) => run_dump(args),
    }
}

fn run_dump(args: DumpArgs) -> Result<(), Box<dyn Error>> {
    let fd = open_pinned_map(&args.map)?;
    let map_fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut writer = match args.output {
        Some(path) => Some(prepare_csv(&path)?),
        None => None,
    };

    for iteration in 0..args.iterations {
        if args.interval > 0 {
            thread::sleep(Duration::from_secs(args.interval));
        }

        let entries = iterate_task_info(map_fd.as_raw_fd())?;
        if entries.is_empty() {
            println!("No task statistics available in the map (is the BPF program loaded?).");
            return Ok(());
        }

        print_table(iteration, &entries);

        if let Some(file) = writer.as_mut() {
            write_csv(file, &entries)?;
        }
    }

    Ok(())
}

fn print_table(iteration: u32, entries: &[(u32, TaskInfo)]) {
    println!("\nIteration {}:", iteration + 1);
    println!(
        "{:<8} {:>12} {:>12} {:>6} {:>7}",
        "PID", "RUNTIME_MS", "SWITCHES", "NICE", "TICKETS"
    );
    for (pid, info) in entries {
        println!(
            "{:<8} {:>12.3} {:>12} {:>6} {:>7}",
            pid,
            info.runtime_ms(),
            info.switches,
            info.nice,
            info.tickets
        );
    }
}

fn prepare_csv(path: &Path) -> io::Result<std::fs::File> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    if file.metadata()?.len() == 0 {
        writeln!(
            file,
            "timestamp_s,pid,runtime_ns,runtime_ms,switches,nice,tickets"
        )?;
    }

    Ok(file)
}

fn write_csv(file: &mut std::fs::File, entries: &[(u32, TaskInfo)]) -> io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    for (pid, info) in entries {
        writeln!(
            file,
            "{:.6},{},{},{:.3},{},{},{}",
            timestamp,
            pid,
            info.runtime_ns,
            info.runtime_ms(),
            info.switches,
            info.nice,
            info.tickets
        )?;
    }

    file.flush()
}
