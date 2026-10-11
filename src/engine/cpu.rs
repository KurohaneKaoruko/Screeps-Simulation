//! CPU 配额状态机: 对齐官方 `@screeps/driver` 的 bucket/tickLimit 语义。
//!
//! - `D/runtime/data.js:149-159`: `Game.cpu.bucket` 显示 `max(0, raw)`;
//!   `tickLimit = min(cpuMaxPerTick=500, max(0, bucket) + limit)`。
//! - `D/runtime/make.js:193-198`: 每 tick 结算 `raw += limit - ceil(cpuUsed)`,
//!   上限 `cpuBucketSize=10000`, **没有下限** —— bucket 可以是负数。
//! - `D/runtime/make.js:72-83`: tick 开始时 `raw < 0` → 跳过本 tick 的用户执行
//!   ("Script execution has been terminated: CPU bucket is empty"),
//!   跳过期间回能 `raw = (raw < -2×limit) ? -2×limit : raw + limit`。
//!   游戏世界照常推进 (只有 bot 的意图缺席)。

/// 官方 `config.engine.cpuMaxPerTick`
pub const CPU_MAX_PER_TICK: f64 = 500.0;
/// 官方 `config.engine.cpuBucketSize`
pub const CPU_BUCKET_SIZE: f64 = 10_000.0;

#[derive(Debug, Clone)]
pub struct CpuState {
    /// 原始 bucket (可为负; 官方 users.cpuAvailable)
    pub raw_bucket: f64,
    /// 每 tick CPU 上限 (官方 users.cpu)
    pub limit: f64,
}

impl CpuState {
    pub fn new(limit: f64, bucket: f64) -> CpuState {
        CpuState { raw_bucket: bucket, limit }
    }

    /// `Game.cpu.bucket` 显示值 (官方 data.js:149-152)
    pub fn displayed_bucket(&self) -> f64 {
        self.raw_bucket.max(0.0)
    }

    /// 本 tick 的 tickLimit: `min(500, max(0, bucket) + limit)`
    pub fn tick_limit(&self) -> f64 {
        (self.displayed_bucket() + self.limit).min(CPU_MAX_PER_TICK)
    }

    /// bucket 是否已耗尽 (负数 → 本 tick 跳过用户执行)
    pub fn should_skip(&self) -> bool {
        self.raw_bucket < 0.0
    }

    /// tick 正常结束: `raw = min(10000, raw + limit - cpuUsed)` (cpuUsed 已按官方 ceil)
    pub fn settle(&mut self, cpu_used: f64) {
        self.raw_bucket = (self.raw_bucket + self.limit - cpu_used).min(CPU_BUCKET_SIZE);
    }

    /// 跳过 tick 时的回能 (官方 make.js:76-79 三目:
    /// `cpuAvailable < -2×limit ? -2×limit : cpuAvailable + limit`)
    pub fn skip_tick(&mut self) {
        self.raw_bucket = if self.raw_bucket < -2.0 * self.limit {
            -2.0 * self.limit
        } else {
            self.raw_bucket + self.limit
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_limit_matches_official_formula() {
        // bucket 充足 → 上限 500
        assert_eq!(CpuState::new(100.0, 10_000.0).tick_limit(), 500.0);
        // bucket 中间值 → bucket + limit (官方语义; 此前误用 max(bucket, limit))
        assert_eq!(CpuState::new(100.0, 50.0).tick_limit(), 150.0);
        assert_eq!(CpuState::new(100.0, 300.0).tick_limit(), 400.0);
        // bucket 为 0 → 恰好 limit
        assert_eq!(CpuState::new(100.0, 0.0).tick_limit(), 100.0);
        // bucket 为负 (跳过 tick 期间) → 显示为 0, tickLimit = limit
        assert_eq!(CpuState::new(20.0, -45.0).tick_limit(), 20.0);
    }

    #[test]
    fn settle_allows_negative_and_caps_at_bucket_size() {
        let mut s = CpuState::new(20.0, 10.0);
        s.settle(50.0); // 10 + 20 - 50 = -20
        assert_eq!(s.raw_bucket, -20.0);
        assert!(s.should_skip());
        assert_eq!(s.displayed_bucket(), 0.0);

        let mut big = CpuState::new(100.0, 9_999.0);
        big.settle(0.0); // 上限 10000
        assert_eq!(big.raw_bucket, 10_000.0);
    }

    #[test]
    fn skip_recovery_floors_at_minus_two_limits() {
        // 深度透支 (低于 -2×limit): 官方三目 `cpuAvailable < -2×cpu ? -2×cpu : +cpu`
        // → 一次跳过直接钳到 -2×limit, 之后每 tick +limit 渐进恢复
        let mut s = CpuState::new(20.0, -100.0);
        s.skip_tick();
        assert_eq!(s.raw_bucket, -40.0, "-100 < -40 → 直接钳到 -2×limit");
        s.skip_tick();
        assert_eq!(s.raw_bucket, -20.0, "此后每 tick +limit");
        s.skip_tick();
        assert_eq!(s.raw_bucket, 0.0);
        assert!(!s.should_skip(), "bucket 恢复到 0 后恢复执行");

        // 浅度透支 (不低于 -2×limit): 按部就班 +limit, 不触发钳制
        let mut s2 = CpuState::new(20.0, -30.0);
        s2.skip_tick();
        assert_eq!(s2.raw_bucket, -10.0, "-30 > -40 → +20 渐进恢复");

        // 边界带 (-3×limit, -2×limit): 官方钳到 -2×limit, 而非 +limit 越过下限
        let mut s3 = CpuState::new(20.0, -50.0);
        s3.skip_tick();
        assert_eq!(s3.raw_bucket, -40.0, "-50 < -40 → 钳到 -2×limit (不是 -30)");
    }
}
