mod bpf_map;
mod stats;
mod tui;

use std::cmp::Ordering;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde::Serialize;
use serde_json::json;

use crate::bpf_map::{iterate_task_info, open_pinned_map};
use crate::stats::{RollingStats, TaskInfo, TaskSnapshot, simulate_lottery_draws, ticket_share};
use crate::tui::{HistorySample, HistoryWindow, draw_dashboard};

#[derive(Serialize)]
#[serde(tag = "ph")]
enum TraceEvent {
    #[serde(rename = "M")]
    Metadata {
        name: &'static str,
        cat: &'static str,
        ts: f64,
        pid: u32,
        tid: u32,
        args: MetadataArgs,
    },
    #[serde(rename = "X")]
    Slice {
        name: String,
        cat: &'static str,
        ts: f64,
        dur: f64,
        pid: u32,
        tid: u32,
        args: TraceArgs,
    },
}

#[derive(Serialize)]
struct MetadataArgs {
    thread_name: String,
}

#[derive(Serialize)]
struct TraceArgs {
    ticket_share: f64,
    deadline_ms: f64,
    lateness_ms: f64,
    runtime_ms: f64,
    utilization: f64,
}

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
    /// Interactive terminal dashboard with live stats
    Tui(TuiArgs),
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

    /// Optional NDJSON output for downstream visualization tools
    #[arg(long)]
    json_output: Option<PathBuf>,

    /// Optional Chrome trace/Perfetto export path
    #[arg(long)]
    trace_output: Option<PathBuf>,

    /// Emit warnings when lateness exceeds this many milliseconds
    #[arg(long, default_value_t = 0.0)]
    deadline_warn: f64,
}

#[derive(Args, Clone)]
struct TuiArgs {
    /// Path to the pinned task map
    #[arg(long, default_value = "/sys/fs/bpf/task_map")]
    map: String,

    /// Refresh period in milliseconds
    #[arg(long, default_value_t = 1000)]
    refresh_ms: u64,

    /// EWMA smoothing factor for rolling runtime (0-1)
    #[arg(long, default_value_t = 0.5)]
    alpha: f64,

    /// How many tasks to show in the dashboard table
    #[arg(long, default_value_t = 10)]
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
        Commands::Tui(args) => run_tui(args),
    }
}

fn run_dump(args: DumpArgs) -> Result<(), Box<dyn Error>> {
    let fd = open_pinned_map(&args.map)?;
    let map_fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut writer = match args.output {
        Some(path) => Some(prepare_csv(&path)?),
        None => None,
    };
    let mut json_writer = match args.json_output {
        Some(path) => Some(prepare_json(&path)?),
        None => None,
    };
    let mut rolling = RollingStats::new(args.alpha);
    let mut rng = match args.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_entropy(),
    };
    let mut trace_events: Vec<TraceEvent> = Vec::new();
    let mut trace_start_ts: Option<f64> = None;

    for iteration in 0..args.iterations {
        if args.interval > 0 {
            thread::sleep(Duration::from_secs(args.interval));
        }

        let entries = iterate_task_info(map_fd.as_raw_fd())?;
        if entries.is_empty() {
            println!("No task statistics available in the map (is the BPF program loaded?).");
            return Ok(());
        }

        let window_ms = if args.interval == 0 {
            1.0
        } else {
            (args.interval as f64).max(0.001) * 1000.0
        };
        let total_tickets: u64 = entries.iter().map(|(_, info)| info.tickets as u64).sum();
        let snapshots = enrich_entries(&entries, total_tickets, &mut rolling, window_ms);
        let timestamp = now_secs();
        if trace_start_ts.is_none() {
            trace_start_ts = Some(timestamp);
        }
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
            print_edf_summary(&ranking, args.top);
        }

        if args.deadline_warn > 0.0 {
            emit_deadline_alerts(args.deadline_warn, &snapshots);
        }

        if let Some(file) = writer.as_mut() {
            write_csv(file, iteration, timestamp, &snapshots)?;
        }
        if let Some(file) = json_writer.as_mut() {
            write_json(file, iteration, timestamp, total_tickets, &snapshots)?;
        }
        if args.trace_output.is_some() {
            let rel_ts = timestamp - trace_start_ts.unwrap_or(timestamp);
            collect_trace_events(&mut trace_events, iteration, rel_ts, &snapshots);
        }
    }

    if let Some(path) = args.trace_output {
        flush_trace(&path, &trace_events)?;
    }

    Ok(())
}

fn run_tui(args: TuiArgs) -> Result<(), Box<dyn Error>> {
    let fd = open_pinned_map(&args.map)?;
    let map_fd = unsafe { OwnedFd::from_raw_fd(fd) };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = tui_loop(&mut terminal, &map_fd, &args);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    map_fd: &OwnedFd,
    args: &TuiArgs,
) -> Result<(), Box<dyn Error>> {
    let mut rolling = RollingStats::new(args.alpha);
    let refresh = Duration::from_millis(args.refresh_ms.max(100));
    let mut history = HistoryWindow::new(120);

    loop {
        let entries = iterate_task_info(map_fd.as_raw_fd())?;
        let total_tickets: u64 = entries.iter().map(|(_, info)| info.tickets as u64).sum();
        let window_ms = refresh.as_secs_f64() * 1000.0;
        let snapshots = enrich_entries(&entries, total_tickets, &mut rolling, window_ms);

        history.push(make_history_sample(&snapshots));

        terminal.draw(|f| {
            draw_dashboard(f, &snapshots, total_tickets, &history, args.top);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                }
            }
        }

        thread::sleep(refresh);
    }

    Ok(())
}

fn make_history_sample(snapshots: &[TaskSnapshot]) -> HistorySample {
    if snapshots.is_empty() {
        return HistorySample::default();
    }

    let (sum_lateness, max_lateness, overdue, runtime_ms, sum_util) = snapshots.iter().fold(
        (0.0_f64, 0.0_f64, 0_usize, 0.0_f64, 0.0_f64),
        |(sum, max, overdue, runtime, util_sum), entry| {
            (
                sum + entry.lateness_ms,
                max.max(entry.lateness_ms),
                overdue + if entry.lateness_ms > 0.0 { 1 } else { 0 },
                runtime + entry.runtime_delta_ms(),
                util_sum + entry.utilization,
            )
        },
    );

    let top = snapshots.iter().max_by(|a, b| {
        a.ticket_share
            .partial_cmp(&b.ticket_share)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let (top_pid, top_share) = top
        .map(|entry| (Some(entry.pid), entry.ticket_share))
        .unwrap_or((None, 0.0));
    let avg_utilization = sum_util / snapshots.len() as f64;

    HistorySample {
        avg_lateness: sum_lateness / snapshots.len() as f64,
        max_lateness,
        total_tasks: snapshots.len(),
        overdue_tasks: overdue,
        total_runtime_ms: runtime_ms,
        avg_utilization,
        top_pid,
        top_share,
    }
}

fn print_table(iteration: u32, total_tickets: u64, entries: &[TaskSnapshot]) {
    println!("\nIteration {}:", iteration + 1);
    println!(
        "{:<8} {:>11} {:>11} {:>11} {:>11} {:>10} {:>8} {:>9} {:>6} {:>8} {:>8}",
        "PID",
        "RUN_MS",
        "DELTA",
        "ROLL",
        "PERIOD",
        "LATENESS",
        "UTIL%",
        "SW_DELTA",
        "NICE",
        "TICKETS",
        "SHARE%"
    );
    for entry in entries {
        println!(
            "{:<8} {:>11.3} {:>11.3} {:>11.3} {:>11.3} {:>10.3} {:>8.2} {:>9} {:>6} {:>8} {:>7.2}",
            entry.pid,
            entry.info.runtime_ms(),
            entry.runtime_delta_ms(),
            entry.rolling_runtime_ms,
            entry.estimated_period_ms,
            entry.lateness_ms,
            entry.utilization * 100.0,
            entry.switch_delta,
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
            "iteration,timestamp_s,pid,runtime_ns,runtime_ms,delta_ns,delta_ms,rolling_runtime_ms,switches,nice,tickets,ticket_share,estimated_period_ms,lateness_ms,utilization"
        )?;
    }

    Ok(file)
}

fn prepare_json(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn write_csv(
    file: &mut std::fs::File,
    iteration: u32,
    timestamp: f64,
    entries: &[TaskSnapshot],
) -> io::Result<()> {
    for entry in entries {
        writeln!(
            file,
            "{},{:.6},{},{},{:.3},{},{:.3},{:.3},{},{},{},{:.6},{:.3},{:.3},{:.3}",
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
            entry.ticket_share,
            entry.estimated_period_ms,
            entry.lateness_ms,
            entry.utilization
        )?;
    }

    file.flush()
}

fn write_json(
    file: &mut std::fs::File,
    iteration: u32,
    timestamp: f64,
    total_tickets: u64,
    entries: &[TaskSnapshot],
) -> io::Result<()> {
    for entry in entries {
        let payload = json!({
            "iteration": iteration + 1,
            "timestamp_s": timestamp,
            "total_tickets": total_tickets,
            "pid": entry.pid,
            "runtime_ms": entry.info.runtime_ms(),
            "delta_ms": entry.runtime_delta_ms(),
            "rolling_runtime_ms": entry.rolling_runtime_ms,
            "switch_delta": entry.switch_delta,
            "estimated_period_ms": entry.estimated_period_ms,
            "deadline_ms": entry.deadline_ms,
            "lateness_ms": entry.lateness_ms,
            "utilization": entry.utilization,
            "nice": entry.info.nice,
            "tickets": entry.info.tickets,
            "ticket_share": entry.ticket_share,
        });
        writeln!(file, "{}", payload)?;
    }
    file.flush()
}

fn enrich_entries(
    entries: &[(u32, TaskInfo)],
    total_tickets: u64,
    rolling: &mut RollingStats,
    window_ms: f64,
) -> Vec<TaskSnapshot> {
    let window_ms = window_ms.max(1.0);
    entries
        .iter()
        .map(|(pid, info)| {
            let (delta_ns, rolling_ms, switch_delta) =
                rolling.update(*pid, info.runtime_ns, info.switches);
            let delta_ms = delta_ns as f64 / 1_000_000.0;
            let mut estimated_period_ms = if switch_delta > 0 {
                window_ms / switch_delta as f64
            } else {
                window_ms
            };
            estimated_period_ms = estimated_period_ms.max(0.1);
            let deadline_ms = estimated_period_ms;
            let lateness_ms = delta_ms - deadline_ms;
            let utilization = if estimated_period_ms > 0.0 {
                delta_ms / estimated_period_ms
            } else {
                0.0
            };
            TaskSnapshot {
                pid: *pid,
                info: *info,
                runtime_delta_ns: delta_ns,
                rolling_runtime_ms: rolling_ms,
                switch_delta,
                estimated_period_ms,
                deadline_ms,
                lateness_ms,
                utilization,
                ticket_share: ticket_share(info.tickets, total_tickets),
            }
        })
        .collect()
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
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

fn print_edf_summary(entries: &[TaskSnapshot], top_n: usize) {
    if entries.is_empty() {
        return;
    }
    let mut ranked = entries.to_vec();
    ranked.sort_by(|a, b| {
        b.lateness_ms
            .partial_cmp(&a.lateness_ms)
            .unwrap_or(Ordering::Equal)
    });
    let limit = ranked.len().min(top_n.max(1));
    println!("\nEDF lateness (top {limit}):");
    let mut any_positive = false;
    for entry in ranked.iter().take(limit) {
        let status = if entry.deadline_missed() {
            "MISS"
        } else {
            "OK"
        };
        println!(
            "{:<8} lateness={:>8.3} ms period={:>8.3} ms util={:>6.2}% share={:>6.2}% status={}",
            entry.pid,
            entry.lateness_ms,
            entry.estimated_period_ms,
            entry.utilization * 100.0,
            entry.ticket_share * 100.0,
            status
        );
        if entry.lateness_ms > 0.0 {
            any_positive = true;
        }
    }
    if !any_positive {
        println!("All sampled tasks met heuristic deadlines in this window.");
    }
}

fn print_draw_results(draws: u32, results: &[(u32, u32)], snapshots: &[TaskSnapshot]) {
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

fn emit_deadline_alerts(threshold_ms: f64, entries: &[TaskSnapshot]) {
    let mut flagged = entries
        .iter()
        .filter(|e| e.lateness_ms > threshold_ms)
        .collect::<Vec<_>>();
    if flagged.is_empty() {
        return;
    }
    flagged.sort_by(|a, b| {
        b.lateness_ms
            .partial_cmp(&a.lateness_ms)
            .unwrap_or(Ordering::Equal)
    });
    println!(
        "\n[!] Deadline alerts (>{:.3} ms over budget):",
        threshold_ms
    );
    for entry in flagged {
        println!(
            "  pid {:>6}: lateness={:>8.3}ms util={:>6.2}% tickets={} nice={}",
            entry.pid,
            entry.lateness_ms,
            entry.utilization * 100.0,
            entry.info.tickets,
            entry.info.nice
        );
    }
}

fn collect_trace_events(
    events: &mut Vec<TraceEvent>,
    _iteration: u32,
    rel_timestamp: f64,
    entries: &[TaskSnapshot],
) {
    let ts_us = rel_timestamp * 1_000_000.0;
    for entry in entries {
        let dur_us = entry.runtime_delta_ms() * 1000.0;
        events.push(TraceEvent::Metadata {
            name: "thread_name",
            cat: "sched",
            ts: 0.0,
            pid: entry.pid,
            tid: entry.pid,
            args: MetadataArgs {
                thread_name: format!("pid {}", entry.pid),
            },
        });
        events.push(TraceEvent::Slice {
            name: format!("pid {}", entry.pid),
            cat: "sched",
            ts: ts_us,
            dur: dur_us.max(1.0),
            pid: entry.pid,
            tid: entry.pid,
            args: TraceArgs {
                ticket_share: entry.ticket_share,
                deadline_ms: entry.deadline_ms,
                lateness_ms: entry.lateness_ms,
                runtime_ms: entry.runtime_delta_ms(),
                utilization: entry.utilization,
            },
        });
    }
}

fn flush_trace(path: &Path, events: &[TraceEvent]) -> Result<(), Box<dyn Error>> {
    if events.is_empty() {
        return Ok(());
    }
    let trace = json!({ "traceEvents": events });
    let data = serde_json::to_string_pretty(&trace)?;
    std::fs::write(path, data)?;
    println!("[+] Trace exported to {}", path.display());
    Ok(())
}
