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
