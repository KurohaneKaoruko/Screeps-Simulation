//! 指标采集与场景断言 (CPU 均值/P95/P99、RCL 时间线、单位数、Memory 峰值)。

use crate::engine::scenario::AssertSpec;
use crate::engine::world::World;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TickMetrics {
    pub tick: u64,
    pub cpu_used: f64,
    pub intents: usize,
    pub creeps: usize,
    pub spawning: usize,
    pub energy_available: u32,
    pub storage_energy: u32,
    pub rcl: u8,
    pub progress: u32,
    pub controller_progress_total: u32,
    pub errors: usize,
    pub console_lines: usize,
}

#[derive(Default)]
pub struct MetricsCollector {
    pub history: Vec<TickMetrics>,
    pub total_errors: usize,
    pub cpu_over_limit_ticks: usize,
    /// 内存字符串大小峰值 (字节)
    pub memory_peak: usize,
}

impl MetricsCollector {
    pub fn record(&mut self, m: TickMetrics, memory_size: usize, over_limit: bool) {
        if over_limit {
            self.cpu_over_limit_ticks += 1;
        }
        self.total_errors += m.errors;
        self.memory_peak = self.memory_peak.max(memory_size);
        self.history.push(m);
    }

    pub fn cpu_avg(&self) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        self.history.iter().map(|m| m.cpu_used).sum::<f64>() / self.history.len() as f64
    }

    pub fn cpu_percentile(&self, p: f64) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        let mut values: Vec<f64> = self.history.iter().map(|m| m.cpu_used).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((values.len() as f64 - 1.0) * p).round() as usize;
        values[idx.min(values.len() - 1)]
    }
}

pub struct AssertResult {
    pub spec: AssertSpec,
    pub passed: bool,
    pub detail: String,
}

pub fn evaluate_asserts(
    world: &World,
    metrics: &MetricsCollector,
    asserts: &[AssertSpec],
) -> Vec<AssertResult> {
    let last = metrics.history.last();
    asserts
        .iter()
        .map(|spec| {
            let (passed, detail) = match spec {
                AssertSpec::RclAtLeast { room, value } => {
                    let level: u8 = match room {
                        Some(name) => world
                            .rooms
                            .get(name)
                            .and_then(|r| r.controller())
                            .map(|c| c.level)
                            .unwrap_or(0),
                        None => world
                            .rooms
                            .values()
                            .filter_map(|r| r.controller())
                            .filter(|c| c.mine)
                            .map(|c| c.level)
                            .max()
                            .unwrap_or(0),
                    };
                    (level >= *value, format!("RCL {level} / 要求 {value}"))
                }
                AssertSpec::RclReachedBy { value, tick } => {
                    let reached = world.rcl_timeline.get(value).copied();
                    let ok = reached.is_some_and(|t| t <= *tick);
                    (ok, format!("RCL{value} 达成于 {reached:?}, 要求 ≤{tick}"))
                }
                AssertSpec::CreepCountAtLeast { value } => {
                    let count = world
                        .rooms
                        .values()
                        .flat_map(|r| r.objects.values())
                        .filter(|o| o.obj_type == "creep" && !o.spawning)
                        .count();
                    (count >= *value, format!("单位 {count} / 要求 {value}"))
                }
                AssertSpec::CpuAvgBelow { value } => {
                    let avg = metrics.cpu_avg();
                    (avg <= *value, format!("CPU 均值 {avg:.3}ms / 上限 {value}"))
                }
                AssertSpec::CpuP95Below { value } => {
                    let p95 = metrics.cpu_percentile(0.95);
                    (p95 <= *value, format!("CPU P95 {p95:.3}ms / 上限 {value}"))
                }
                AssertSpec::NoErrors => (
                    metrics.total_errors == 0,
                    format!("错误 tick 数 {}", metrics.total_errors),
                ),
            };
            let _ = last;
            AssertResult {
                spec: spec.clone(),
                passed,
                detail,
            }
        })
        .collect()
}
