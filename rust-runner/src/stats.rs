use std::collections::HashMap;

use rand::Rng;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct TaskInfo {
    pub runtime_ns: u64,
    pub switches: u64,
    pub nice: i32,
    pub tickets: u32,
    pub last_switch_in_ts: u64,
}

impl TaskInfo {
    pub fn runtime_ms(&self) -> f64 {
        self.runtime_ns as f64 / 1_000_000.0
    }
}

#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    pub pid: u32,
    pub info: TaskInfo,
    pub runtime_delta_ns: u64,
    pub rolling_runtime_ms: f64,
    pub ticket_share: f64,
}

impl TaskSnapshot {
    pub fn runtime_delta_ms(&self) -> f64 {
        self.runtime_delta_ns as f64 / 1_000_000.0
    }
}

#[derive(Debug)]
pub struct RollingStats {
    alpha: f64,
    prev_runtime_ns: HashMap<u32, u64>,
    rolling_runtime_ms: HashMap<u32, f64>,
}

impl RollingStats {
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            prev_runtime_ns: HashMap::new(),
            rolling_runtime_ms: HashMap::new(),
        }
    }

    pub fn update(&mut self, pid: u32, runtime_ns: u64) -> (u64, f64) {
        let prev = self.prev_runtime_ns.insert(pid, runtime_ns);
        let delta_ns = prev
            .map(|p| runtime_ns.saturating_sub(p))
            .unwrap_or_default();
        let delta_ms = delta_ns as f64 / 1_000_000.0;
        let current = self.rolling_runtime_ms.entry(pid).or_insert(delta_ms);
        let next = self.alpha * delta_ms + (1.0 - self.alpha) * *current;
        *current = next;
        (delta_ns, next)
    }
}

pub fn ticket_share(tickets: u32, total_tickets: u64) -> f64 {
    if total_tickets == 0 {
        0.0
    } else {
        tickets as f64 / total_tickets as f64
    }
}

pub fn simulate_lottery_draws<R: Rng + ?Sized>(
    rng: &mut R,
    population: &[TaskSnapshot],
    draws: u32,
) -> Vec<(u32, u32)> {
    let total_tickets: u64 = population.iter().map(|s| s.info.tickets as u64).sum();
    if draws == 0 || total_tickets == 0 {
        return Vec::new();
    }

    let mut counts: HashMap<u32, u32> = HashMap::new();
    for _ in 0..draws {
        let mut target = rng.gen_range(0..total_tickets);
        for snap in population {
            let share = snap.info.tickets as u64;
            if share == 0 {
                continue;
            }

            if target < share {
                *counts.entry(snap.pid).or_insert(0) += 1;
                break;
            } else {
                target -= share;
            }
        }
    }

    let mut pairs: Vec<(u32, u32)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));
    pairs
}
