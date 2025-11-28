use std::collections::VecDeque;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table},
};

use crate::stats::TaskSnapshot;

pub struct HistoryWindow {
    capacity: usize,
    samples: VecDeque<HistorySample>,
}

#[derive(Clone, Default)]
pub struct HistorySample {
    pub avg_lateness: f64,
    pub max_lateness: f64,
    pub total_tasks: usize,
    pub overdue_tasks: usize,
    pub total_runtime_ms: f64,
    pub avg_utilization: f64,
    pub top_pid: Option<u32>,
    pub top_share: f64,
}

impl HistoryWindow {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            samples: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    pub fn push(&mut self, sample: HistorySample) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn iter(&self) -> impl Iterator<Item = &HistorySample> {
        self.samples.iter()
    }

    pub fn latest(&self) -> Option<&HistorySample> {
        self.samples.back()
    }
}

fn render_table(frame: &mut Frame<'_>, snapshots: &[TaskSnapshot], top_n: usize, area: Rect) {
    let mut ranking = snapshots.to_vec();
    ranking.sort_by(|a, b| {
        b.ticket_share
            .partial_cmp(&a.ticket_share)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let limit = ranking.len().min(top_n.max(1));

    let header = Row::new(vec![
        "PID",
        "SHARE%",
        "LAT(ms)",
        "UTIL%",
        "DELTA (ms)",
        "PERIOD (ms)",
        "TICKETS",
        "NICE",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = ranking
        .iter()
        .take(limit)
        .map(|entry| {
            let cells = vec![
                entry.pid.to_string(),
                format!("{:.2}", entry.ticket_share * 100.0),
                format!("{:.3}", entry.lateness_ms),
                format!("{:.1}", entry.utilization * 100.0),
                format!("{:.3}", entry.runtime_delta_ms()),
                format!("{:.3}", entry.estimated_period_ms),
                entry.info.tickets.to_string(),
                entry.info.nice.to_string(),
            ];
            let mut row = Row::new(cells);
            if entry.lateness_ms > 0.0 {
                row = row.style(Style::default().fg(Color::Red));
            }
            row
        })
        .collect();

    let widths = [
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(6),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().title("Top tasks").borders(Borders::ALL));
    frame.render_widget(table, area);
}

fn render_summary(frame: &mut Frame<'_>, history: &HistoryWindow, total_tickets: u64, area: Rect) {
    let latest = history.latest().cloned().unwrap_or_default();
    let top_line = match latest.top_pid {
        Some(pid) => format!("Top pid {pid} ({:.1}% share)", latest.top_share * 100.0),
        None => "Top pid n/a".to_string(),
    };
    let status = format!(
        "Tasks: {tasks}  Tickets: {tickets}  Avg lateness: {avg:.3} ms  Worst: {max:.3} ms  Avg util: {util:.1}%\nOverdue: {overdue}  Runtime window: {runtime:.3} ms  {top_line}  Press q/Esc to exit",
        tasks = latest.total_tasks,
        tickets = total_tickets,
        avg = latest.avg_lateness,
        max = latest.max_lateness.max(0.0_f64),
        util = latest.avg_utilization * 100.0,
        overdue = latest.overdue_tasks,
        runtime = latest.total_runtime_ms,
    );
    let block =
        Paragraph::new(status).block(Block::default().title("Summary").borders(Borders::ALL));
    frame.render_widget(block, area);
}

fn render_history(frame: &mut Frame<'_>, history: &HistoryWindow, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Length(5),
            ]
            .as_ref(),
        )
        .split(area);

    render_metric_sparkline(
        frame,
        sections[0],
        history,
        |s| s.avg_lateness.max(0.0),
        1000.0,
        "Avg lateness trend (ms)",
        Color::Cyan,
    );

    render_metric_sparkline(
        frame,
        sections[1],
        history,
        |s| s.max_lateness.max(0.0),
        1000.0,
        "Worst lateness trend (ms)",
        Color::LightMagenta,
    );

    render_metric_sparkline(
        frame,
        sections[2],
        history,
        |s| (s.avg_utilization * 100.0).clamp(0.0, 200.0),
        1.0,
        "Avg utilisation trend (%)",
        Color::Yellow,
    );

    render_metric_sparkline(
        frame,
        sections[3],
        history,
        |s| s.overdue_tasks as f64,
        1.0,
        "Overdue tasks",
        Color::Red,
    );

    render_metric_sparkline(
        frame,
        sections[4],
        history,
        |s| s.total_runtime_ms.max(0.0),
        1.0,
        "Runtime window (ms)",
        Color::Green,
    );

    let latest = history.latest().cloned().unwrap_or_default();
    let text = format!(
        "Latest avg: {avg:.3} ms  Worst: {max:.3} ms  Tasks: {tasks}  Overdue: {overdue}",
        avg = latest.avg_lateness,
        max = latest.max_lateness.max(0.0_f64),
        tasks = latest.total_tasks,
        overdue = latest.overdue_tasks,
    );
    let footer =
        Paragraph::new(text).block(Block::default().title("Trend stats").borders(Borders::ALL));
    frame.render_widget(footer, sections[5]);

    let ascii_lines = [
        " _______________________ ",
        "|  ___| ___ \\ ___ \\  ___|",
        "| |__ | |_/ / |_/ / |_   ",
        "|  __|| ___ \\  __/|  _|  ",
        "| |___| |_/ / |   | |    ",
        "\\____/\\____/\\_|   \\_|    ",
    ];
    let ascii_width = ascii_lines.iter().map(|l| l.len()).max().unwrap_or(0) as u16;
    let ascii_height = ascii_lines.len() as u16;
    let offset_x = sections[6]
        .width
        .saturating_sub(ascii_width)
        .checked_div(2)
        .unwrap_or(0);
    let offset_y = sections[6]
        .height
        .saturating_sub(ascii_height)
        .checked_div(2)
        .unwrap_or(0);
    let art_area = Rect {
        x: sections[6].x + offset_x,
        y: sections[6].y + offset_y,
        width: ascii_width.min(sections[6].width),
        height: 15,
    };
    let art = Paragraph::new(ascii_lines.join("\n")).style(Style::default().fg(Color::Blue));
    let block = Block::default().title("LOGO").borders(Borders::ALL);
    frame.render_widget(art.block(block), art_area);
}

fn render_metric_sparkline<F>(
    frame: &mut Frame<'_>,
    area: Rect,
    history: &HistoryWindow,
    projection: F,
    scale: f64,
    title: &str,
    color: Color,
) where
    F: Fn(&HistorySample) -> f64,
{
    if history.samples.len() < 2 {
        let block = Paragraph::new("Collecting history...")
            .block(Block::default().title(title).borders(Borders::ALL));
        frame.render_widget(block, area);
        return;
    }

    let data: Vec<u64> = history
        .iter()
        .map(|sample| (projection(sample) * scale).max(0.0) as u64)
        .collect();
    let max_val = data.iter().copied().max().unwrap_or(0).max(1);
    let spark = Sparkline::default()
        .block(Block::default().title(title).borders(Borders::ALL))
        .style(Style::default().fg(color))
        .max(max_val)
        .data(&data);
    frame.render_widget(spark, area);
}
pub fn draw_dashboard(
    frame: &mut Frame<'_>,
    snapshots: &[TaskSnapshot],
    total_tickets: u64,
    history: &HistoryWindow,
    top_n: usize,
) {
    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
        .split(frame.size());

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(5)].as_ref())
        .split(main_layout[0]);

    render_table(frame, snapshots, top_n, left_chunks[0]);
    render_summary(frame, history, total_tickets, left_chunks[1]);

    render_history(frame, history, main_layout[1]);
}
