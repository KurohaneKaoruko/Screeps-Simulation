//! 时间旅行: 世界检查点 (checkpoint) 的落盘/恢复。
//!
//! 与 `--record-intents` 的意图录制配合使用:
//! - 记录阶段: 每 N tick 把 World + Memory + CPU bucket 落盘 `cp-{tick}.json`;
//! - 回放阶段: `--replay intents.jsonl --at T` 恢复 ≤T 的最近检查点, 重放其余意图到 tick T。
//!
//! 检查点是**确定性**的: 同一场景同 seed 下, 从检查点恢复重放到 tick T 的世界
//! 与连续运行到 T 的世界逐位一致 (状态哈希相同)。

use crate::engine::world::World;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 检查点文件名前缀 (完整格式 `cp-{tick:06}.json`, tick = 已处理的 tick 数 = world.tick)
pub const CHECKPOINT_PREFIX: &str = "cp-";

/// 落盘的检查点记录: 世界状态 + 恢复运行所需的宿主状态
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    /// 世界当前 tick (已处理的 tick 数; 下一条要应用的录制意图的 tick 号)
    pub tick: u64,
    pub world: World,
    /// 当前 Memory JSON (续跑时注入 JS 宿主)
    pub memory: String,
    /// CPU bucket 原始值 (可为负; 官方 users.cpuAvailable)
    pub cpu_bucket: f64,
    /// CPU limit (官方 users.cpu)
    pub cpu_limit: f64,
}

impl Checkpoint {
    /// 从当前世界状态构造检查点 (World 克隆一份; 检查点只在 save 时序列化)
    pub fn capture(world: &World, memory: &str, cpu_bucket: f64, cpu_limit: f64) -> Checkpoint {
        Checkpoint {
            tick: world.tick,
            world: world.clone(),
            memory: memory.to_string(),
            cpu_bucket,
            cpu_limit,
        }
    }

    /// 恢复世界 (terrain_json 缓存随同刷新)
    pub fn restore_world(self) -> World {
        let mut world = self.world;
        for room in world.rooms.values_mut() {
            room.ensure_terrain_cache();
        }
        world
    }
}

/// 保存检查点到 `dir/cp-{tick:06}.json` (原子替换: 先写临时文件再 rename)
pub fn save(dir: &Path, tick: u64, cp: &Checkpoint) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = checkpoint_path(dir, tick);
    let tmp = dir.join(format!("cp-{tick:06}.json.tmp"));
    let text = serde_json::to_string(cp).expect("检查点序列化失败");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// 检查点文件路径 (tick = world.tick, 已处理 tick 数)
pub fn checkpoint_path(dir: &Path, tick: u64) -> PathBuf {
    dir.join(format!("{CHECKPOINT_PREFIX}{tick:06}.json"))
}

/// 从文件名解析 tick (`cp-000123.json` → 123); 不符格式返回 None
pub fn parse_checkpoint_tick(file_name: &str) -> Option<u64> {
    let rest = file_name.strip_prefix(CHECKPOINT_PREFIX)?.strip_suffix(".json")?;
    rest.parse().ok()
}

/// 在目录中找 tick ≤ at 的最近检查点 (at = "目标 tick", 即期望恢复到的 world.tick)
pub fn latest_checkpoint(dir: &Path, at: u64) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(tick) = parse_checkpoint_tick(name) else { continue };
        if tick <= at && best.as_ref().is_none_or(|(t, _)| tick > *t) {
            best = Some((tick, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// 读取检查点
pub fn load(path: &Path) -> std::io::Result<Checkpoint> {
    let text = std::fs::read_to_string(path)?;
    let cp: Checkpoint =
        serde_json::from_str(&text).map_err(|e| std::io::Error::other(format!("检查点解析失败: {e}")))?;
    Ok(cp)
}
