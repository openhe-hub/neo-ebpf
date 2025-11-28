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
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::bpf_map::{iterate_task_info, open_pinned_map};
use crate::stats::{
    simulate_lottery_draws, ticket_share, RollingStats, TaskInfo, TaskSnapshot,
};

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

    /// Number of simulated lottery draws per iteration
    #[arg(long, default_value_t = 0)]
    simulate_draws: u32,

    /// EWMA smoothing factor for rolling runtime (0-1)
    #[arg(long, default_value_t = 0.5)]
    alpha: f64,

    /// Optional RNG seed for reproducible lottery draws
    #[arg(long)]
    seed: Option<u64>,

    /// How many top tasks to display in the lottery summary
    #[arg(long, default_value_t = 5)]
    top: usize,
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
    let mut rolling = RollingStats::new(args.alpha);
    let mut rng = match args.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_entropy(),
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

        let total_tickets: u64 = entries.iter().map(|(_, info)| info.tickets as u64).sum();
        let snapshots = enrich_entries(&entries, total_tickets, &mut rolling);
        print_table(iteration, total_tickets, &snapshots);

        if !snapshots.is_empty() {
            let mut ranking = snapshots.clone();
            ranking.sort_by(|a, b| {
                b.ticket_share
                    .partial_cmp(&a.ticket_share)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            print_lottery_summary(&ranking, args.top);
            if args.simulate_draws > 0 {
                let draws = simulate_lottery_draws(&mut rng, &ranking, args.simulate_draws);
                print_draw_results(args.simulate_draws, &draws, &ranking);
            }
        }

        if let Some(file) = writer.as_mut() {
            write_csv(file, iteration, &snapshots)?;
        }
    }

    Ok(())
}

fn print_table(iteration: u32, total_tickets: u64, entries: &[TaskSnapshot]) {
    println!("\nIteration {}:", iteration + 1);
    println!(
        "{:<8} {:>12} {:>12} {:>12} {:>10} {:>6} {:>8} {:>8}",
        "PID",
        "RUNTIME_MS",
        "DELTA_MS",
        "ROLL_MS",
        "SWITCHES",
        "NICE",
        "TICKETS",
        "SHARE%"
    );
    for entry in entries {
        println!(
            "{:<8} {:>12.3} {:>12.3} {:>12.3} {:>10} {:>6} {:>8} {:>7.2}",
            entry.pid,
            entry.info.runtime_ms(),
            entry.runtime_delta_ms(),
            entry.rolling_runtime_ms,
            entry.info.switches,
            entry.info.nice,
            entry.info.tickets,
            entry.ticket_share * 100.0
        );
    }
    if total_tickets == 0 {
        println!("Total tickets: 0 (all tasks currently inactive).");
    } else {
        println!("Total tickets: {total_tickets}");
    }
}

fn prepare_csv(path: &Path) -> io::Result<std::fs::File> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    if file.metadata()?.len() == 0 {
        writeln!(
            file,
            "iteration,timestamp_s,pid,runtime_ns,runtime_ms,delta_ns,delta_ms,rolling_runtime_ms,switches,nice,tickets,ticket_share"
        )?;
    }

    Ok(file)
}

fn write_csv(file: &mut std::fs::File, iteration: u32, entries: &[TaskSnapshot]) -> io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    for entry in entries {
        writeln!(
            file,
            "{},{:.6},{},{},{:.3},{},{:.3},{:.3},{},{},{},{:.6}",
            iteration + 1,
            timestamp,
            entry.pid,
            entry.info.runtime_ns,
            entry.info.runtime_ms(),
            entry.runtime_delta_ns,
            entry.runtime_delta_ms(),
            entry.rolling_runtime_ms,
            entry.info.switches,
            entry.info.nice,
            entry.info.tickets,
            entry.ticket_share
        )?;
    }

    file.flush()
}

fn enrich_entries(
    entries: &[(u32, TaskInfo)],
    total_tickets: u64,
    rolling: &mut RollingStats,
) -> Vec<TaskSnapshot> {
    entries
        .iter()
        .map(|(pid, info)| {
            let (delta_ns, rolling_ms) = rolling.update(*pid, info.runtime_ns);
            TaskSnapshot {
                pid: *pid,
                info: *info,
                runtime_delta_ns: delta_ns,
                rolling_runtime_ms: rolling_ms,
                ticket_share: ticket_share(info.tickets, total_tickets),
            }
        })
        .collect()
}

fn print_lottery_summary(entries: &[TaskSnapshot], top_n: usize) {
    if entries.is_empty() {
        return;
    }
    let limit = entries.len().min(top_n.max(1));
    println!("\nTop {} candidates by ticket share:", limit);
    println!("{:<8} {:>10} {:>9}", "PID", "TICKETS", "SHARE%");
    for entry in entries.iter().take(limit) {
        println!(
            "{:<8} {:>10} {:>8.2}",
            entry.pid,
            entry.info.tickets,
            entry.ticket_share * 100.0
        );
    }
}

fn print_draw_results(
    draws: u32,
    results: &[(u32, u32)],
    snapshots: &[TaskSnapshot],
) {
    if draws == 0 {
        return;
    }

    if results.is_empty() {
        println!("\nLottery simulation skipped (no tickets present).");
        return;
    }

    println!("\nLottery simulation ({} draws):", draws);
    println!(
        "{:<8} {:>10} {:>11} {:>11}",
        "PID", "WINS", "WIN_RATE%", "SHARE%"
    );

    for (pid, count) in results {
        let win_rate = (*count as f64 / draws as f64) * 100.0;
        let share = snapshots
            .iter()
            .find(|snap| snap.pid == *pid)
            .map(|snap| snap.ticket_share * 100.0)
            .unwrap_or(0.0);
        println!(
            "{:<8} {:>10} {:>11.2} {:>11.2}",
            pid, count, win_rate, share
        );
    }
}
