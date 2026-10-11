//! 官方常量表 (docs.screeps.com Constants + screeps/engine 驱动常量核对)。
//! 数值即事实来源, 与线上服务器一致, 不得随意改动。

/// 部件成本
pub const BODYPART_COST: &[(&str, u32)] = &[
    ("move", 50),
    ("work", 100),
    ("carry", 50),
    ("attack", 80),
    ("ranged_attack", 150),
    ("tough", 10),
    ("heal", 250),
    ("claim", 600),
];

pub fn bodypart_cost(part: &str) -> u32 {
    BODYPART_COST
        .iter()
        .find(|(p, _)| *p == part)
        .map(|(_, c)| *c)
        .unwrap_or(0)
}

pub const MAX_CREEP_SIZE: usize = 50;

/// 生命周期与功率
pub const CREEP_LIFE_TIME: u64 = 1500;
pub const CREEP_CLAIM_LIFE_TIME: u64 = 600;
pub const CREEP_CORPSE_RATE: f64 = 0.2;
pub const CREEP_PART_MAX_ENERGY: u32 = 125;
pub const CARRY_CAPACITY: u32 = 50;
pub const HARVEST_POWER: u32 = 2;
pub const BUILD_POWER: u32 = 5;
pub const REPAIR_POWER: u32 = 100;
pub const UPGRADE_CONTROLLER_POWER: u32 = 1;
pub const REPAIR_COST: f64 = 0.01;
pub const TOMBSTONE_DECAY_PER_PART: u64 = 5;

/// Spawn
pub const SPAWN_HITS: u32 = 5000;
pub const SPAWN_ENERGY_START: u32 = 300;
pub const SPAWN_ENERGY_CAPACITY: u32 = 300;
pub const CREEP_SPAWN_TIME: u64 = 3;

/// Source / 掉落能量
pub const SOURCE_ENERGY_CAPACITY: u32 = 3000;
pub const SOURCE_ENERGY_NEUTRAL_CAPACITY: u32 = 1500;
pub const ENERGY_REGEN_TIME: u64 = 300;
/// 掉落能量每 tick 衰减 ceil(amount / ENERGY_DECAY)
pub const ENERGY_DECAY: u32 = 1000;

/// Controller 升级进度需求 (当前等级 → 下一级)
pub const CONTROLLER_LEVELS: &[(u8, u32)] = &[
    (1, 200),
    (2, 45_000),
    (3, 135_000),
    (4, 405_000),
    (5, 1_215_000),
    (6, 3_645_000),
    (7, 10_935_000),
];

/// 降级计时 (按等级)
pub const CONTROLLER_DOWNGRADE: &[(u8, u64)] = &[
    (1, 20_000),
    (2, 10_000),
    (3, 20_000),
    (4, 40_000),
    (5, 80_000),
    (6, 120_000),
    (7, 150_000),
    (8, 200_000),
];

/// 升级后降级计时恢复量
pub const CONTROLLER_DOWNGRADE_RESTORE: u64 = 100;
/// RCL 8 时每 tick 最多注入的升级能量
pub const CONTROLLER_MAX_UPGRADE_PER_TICK: u32 = 15;

/// GCL: 升到下一级所需 = GCL_MULTIPLY * GCL_POW^(level-1)
pub const GCL_POW: f64 = 2.4;
pub const GCL_MULTIPLY: u64 = 1_000_000;

/// Extension 容量按 RCL
pub fn extension_capacity(rcl: u8) -> u32 {
    match rcl {
        0..=6 => 50,
        7 => 100,
        _ => 200,
    }
}

/// 建造成本
pub fn construction_cost(structure_type: &str) -> Option<u32> {
    Some(match structure_type {
        "spawn" => 15_000,
        "extension" => 3_000,
        "road" => 300,
        "constructedWall" => 1,
        "rampart" => 1,
        "link" => 5_000,
        "storage" => 30_000,
        "tower" => 5_000,
        "observer" => 8_000,
        "powerSpawn" => 100_000,
        "extractor" => 5_000,
        "lab" => 50_000,
        "terminal" => 100_000,
        "container" => 5_000,
        "nuker" => 100_000,
        "factory" => 100_000,
        _ => return None,
    })
}

pub const CONSTRUCTION_COST_ROAD_SWAMP_RATIO: u32 = 5;

/// RCL 对应的可建数量上限 (CONTROLLER_STRUCTURES 精简版: 只列经济期会用到的)
pub fn controller_structure_limit(structure_type: &str, rcl: u8) -> u32 {
    match structure_type {
        "spawn" => [1, 1, 1, 1, 1, 1, 1, 2, 3][rcl.min(8) as usize],
        "extension" => [0, 0, 5, 10, 20, 30, 40, 50, 60][rcl.min(8) as usize],
        "road" => 2500,
        "constructedWall" => [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500][rcl.min(8) as usize],
        "rampart" => [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500][rcl.min(8) as usize],
        "link" => [0, 0, 0, 0, 0, 2, 3, 4, 6][rcl.min(8) as usize],
        "storage" => [0, 0, 0, 1, 1, 1, 1, 1, 1][rcl.min(8) as usize],
        "tower" => [0, 0, 0, 1, 1, 2, 2, 3, 6][rcl.min(8) as usize],
        "observer" => [0, 0, 0, 0, 0, 0, 0, 0, 1][rcl.min(8) as usize],
        "powerSpawn" => [0, 0, 0, 0, 0, 0, 0, 1, 1][rcl.min(8) as usize],
        "extractor" => [0, 0, 0, 0, 0, 1, 1, 1, 1][rcl.min(8) as usize],
        "lab" => [0, 0, 0, 0, 0, 3, 6, 10, 10][rcl.min(8) as usize],
        "terminal" => [0, 0, 0, 0, 0, 1, 1, 1, 1][rcl.min(8) as usize],
        "container" => 5,
        "nuker" => [0, 0, 0, 0, 0, 0, 0, 0, 1][rcl.min(8) as usize],
        "factory" => [0, 0, 0, 0, 0, 0, 0, 1, 1][rcl.min(8) as usize],
        _ => 0,
    }
}

/// 阻挡移动的对象类型 (OBSTACLE_OBJECT_TYPES)
pub const OBSTACLE_OBJECT_TYPES: &[&str] = &[
    "spawn",
    "creep",
    "constructedWall",
    "extension",
    "link",
    "storage",
    "tower",
    "observer",
    "powerSpawn",
    "lab",
    "factory",
    "nuker",
    "keeperLair",
    "source",
    "mineral",
    "deposit",
    "invaderCore",
];

/// 结构耐久
pub fn structure_hits(structure_type: &str) -> u32 {
    match structure_type {
        "spawn" => SPAWN_HITS,
        "extension" => 1000,
        "road" => 5000,
        "constructedWall" => 1,
        "rampart" => 1,
        "link" => 1000,
        "storage" => 10_000,
        "tower" => 3000,
        "observer" => 500,
        "powerSpawn" => 5000,
        "extractor" => 500,
        "lab" => 500,
        "terminal" => 3000,
        "container" => 250_000,
        "nuker" => 1000,
        "factory" => 1000,
        _ => 0,
    }
}

pub const WALL_HITS_MAX: u32 = 300_000_000;
pub const STORAGE_CAPACITY: u32 = 1_000_000;
pub const CONTAINER_CAPACITY: u32 = 2000;
pub const TOWER_CAPACITY: u32 = 1000;
pub const TERMINAL_CAPACITY: u32 = 300_000;

/// 容器衰减: 每 CONTAINER_DECAY_TIME 掉 CONTAINER_DECAY 血 (自有房间 500t)
pub const CONTAINER_DECAY: u32 = 5000;
pub const CONTAINER_DECAY_TIME: u64 = 100;
pub const CONTAINER_DECAY_TIME_OWNED: u64 = 500;

/// 道路衰减: 每 1000t 掉 100 血 (沼泽路 ×5)
pub const ROAD_DECAY_AMOUNT: u32 = 100;
pub const ROAD_DECAY_TIME: u64 = 1000;
pub const ROAD_HITS: u32 = 5000;
pub const ROAD_WEAROUT: i64 = 1;

/// 地形掩码
pub const TERRAIN_MASK_WALL: u8 = 1;
pub const TERRAIN_MASK_SWAMP: u8 = 2;

/// 每个玩家的最大工地数
pub const MAX_CONSTRUCTION_SITES: usize = 100;

/// GCL 等级 → 升到下一级所需总进度 (1 基)
pub fn gcl_progress_total(level: u32) -> u64 {
    (GCL_MULTIPLY as f64 * GCL_POW.powi(level as i32 - 1)) as u64
}
