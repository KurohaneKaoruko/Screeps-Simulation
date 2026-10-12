//! 官方常量表 (docs.screeps.com Constants + screeps/engine 驱动常量核对)。
//! 数值即事实来源, 与线上服务器一致, 不得随意改动。
#![allow(dead_code)]

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

/// 合法身体部件名 (官方 BODYPART_COST 的键集合)
pub fn is_known_bodypart(part: &str) -> bool {
    BODYPART_COST.iter().any(|(p, _)| *p == part)
}

/// 场景可放置的结构类型 (structure_hits 覆盖的集合 + 控制器/传送门等特殊类型)
pub fn is_known_structure_type(t: &str) -> bool {
    matches!(
        t,
        "spawn"
            | "extension"
            | "road"
            | "constructedWall"
            | "rampart"
            | "link"
            | "storage"
            | "tower"
            | "observer"
            | "powerSpawn"
            | "extractor"
            | "lab"
            | "terminal"
            | "container"
            | "nuker"
            | "factory"
            | "keeperLair"
            | "powerBank"
            | "portal"
            | "controller"
            | "invaderCore"
    )
}

pub const MAX_CREEP_SIZE: usize = 50;

/// 官方 RESOURCES_ALL 顺序 (common/lib/constants.js)。
/// 用于 `_drop-resources-without-space` 等需要固定资源枚举顺序的结算。
pub const RESOURCES_ALL: &[&str] = &[
    "energy",
    "power",
    "H",
    "O",
    "U",
    "K",
    "L",
    "Z",
    "X",
    "G",
    "OH",
    "ZK",
    "UL",
    "UH",
    "UO",
    "KH",
    "KO",
    "LH",
    "LO",
    "ZH",
    "ZO",
    "GH",
    "GO",
    "UH2O",
    "UHO2",
    "KH2O",
    "KHO2",
    "LH2O",
    "LHO2",
    "ZH2O",
    "ZHO2",
    "GH2O",
    "GHO2",
    "XUH2O",
    "XUHO2",
    "XKH2O",
    "XKHO2",
    "XLH2O",
    "XLHO2",
    "XZH2O",
    "XZHO2",
    "XGH2O",
    "XGHO2",
    "ops",
    "silicon",
    "metal",
    "biomass",
    "mist",
    "utrium_bar",
    "lemergium_bar",
    "zynthium_bar",
    "keanium_bar",
    "ghodium_melt",
    "oxidant",
    "reductant",
    "purifier",
    "battery",
    "composite",
    "crystal",
    "liquid",
    "wire",
    "switch",
    "transistor",
    "microchip",
    "circuit",
    "device",
    "cell",
    "phlegm",
    "tissue",
    "muscle",
    "organoid",
    "organism",
    "alloy",
    "tube",
    "fixtures",
    "frame",
    "hydraulics",
    "machine",
    "condensate",
    "concentrate",
    "extract",
    "spirit",
    "emanation",
    "essence",
];

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
/// 官方 SPAWN_RENEW_RATIO: renew 的寿命恢复/能量消耗系数
pub const SPAWN_RENEW_RATIO: f64 = 1.2;

/// Tower (官方 TOWER_*)
pub const TOWER_ENERGY_COST: u32 = 10;
pub const TOWER_POWER_ATTACK: f64 = 600.0;
pub const TOWER_POWER_HEAL: f64 = 400.0;
pub const TOWER_POWER_REPAIR: f64 = 800.0;
pub const TOWER_OPTIMAL_RANGE: f64 = 5.0;
pub const TOWER_FALLOFF_RANGE: f64 = 20.0;
pub const TOWER_FALLOFF: f64 = 0.75;

/// Link (官方 LINK_*)
pub const LINK_COOLDOWN: u32 = 1;

/// ── 战斗 (官方 constants.js 逐条对齐) ──
/// 每件部件的基础功率 (无 boost 加成)
pub const ATTACK_POWER: u32 = 30;
pub const RANGED_ATTACK_POWER: u32 = 10;
pub const HEAL_POWER: u32 = 12;
pub const RANGED_HEAL_POWER: u32 = 4;
pub const DISMANTLE_POWER: u32 = 50;
/// 拆除收益系数 (每 1 点拆除伤害回收 floor(amount × DISMANTLE_COST) 能量)
pub const DISMANTLE_COST: f64 = 0.005;
/// rangedMassAttack 的距离衰减 (官方 processor/intents/creeps/rangedMassAttack.js: distanceRate)
pub const RANGED_MASS_ATTACK_DISTANCE_RATE: &[(i32, f64)] = &[(0, 1.0), (1, 1.0), (2, 0.4), (3, 0.1)];

/// 控制器保留 / 攻击相关 (官方 CONTROLLER_*)
pub const CONTROLLER_RESERVE: u64 = 1;
pub const CONTROLLER_RESERVE_MAX: u64 = 5000;
pub const CONTROLLER_CLAIM_DOWNGRADE: u64 = 300;
pub const CONTROLLER_ATTACK_BLOCKED_UPGRADE: u64 = 1000;

/// 官方 EVENT_ATTACK_TYPE_* / EVENT_HEAL_TYPE_*
pub const EVENT_ATTACK_TYPE_MELEE: u32 = 1;
pub const EVENT_ATTACK_TYPE_RANGED: u32 = 2;
pub const EVENT_ATTACK_TYPE_RANGED_MASS: u32 = 3;
pub const EVENT_ATTACK_TYPE_DISMANTLE: u32 = 4;
pub const EVENT_ATTACK_TYPE_HIT_BACK: u32 = 5;
pub const EVENT_ATTACK_TYPE_NUKE: u32 = 6;
pub const EVENT_HEAL_TYPE_MELEE: u32 = 1;
pub const EVENT_HEAL_TYPE_RANGED: u32 = 2;

/// Source / 掉落能量
pub const SOURCE_ENERGY_CAPACITY: u32 = 3000;
pub const SOURCE_ENERGY_NEUTRAL_CAPACITY: u32 = 1500;
pub const SOURCE_ENERGY_KEEPER_CAPACITY: u32 = 4000;
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

/// RCL 对应的可建数量上限 (CONTROLLER_STRUCTURES, 逐级照抄官方 K:213-229)
///
/// 索引即 RCL (0..=8)。注意 RCL0 不含任何 spawn, storage 自 RCL4, terminal/extractor
/// 自 RCL6, lab 3/6/10 位于 RCL6/7/8, powerSpawn 位于 RCL8。
pub fn controller_structure_limit(structure_type: &str, rcl: u8) -> u32 {
    let table: [u32; 9] = match structure_type {
        "spawn" => [0, 1, 1, 1, 1, 1, 1, 2, 3],
        "extension" => [0, 0, 5, 10, 20, 30, 40, 50, 60],
        "road" => [2500; 9],
        "constructedWall" => [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500],
        "rampart" => [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500],
        "link" => [0, 0, 0, 0, 0, 2, 3, 4, 6],
        "storage" => [0, 0, 0, 0, 1, 1, 1, 1, 1],
        "tower" => [0, 0, 0, 1, 1, 2, 2, 3, 6],
        "observer" => [0, 0, 0, 0, 0, 0, 0, 0, 1],
        "powerSpawn" => [0, 0, 0, 0, 0, 0, 0, 0, 1],
        "extractor" => [0, 0, 0, 0, 0, 0, 1, 1, 1],
        "lab" => [0, 0, 0, 0, 0, 0, 3, 6, 10],
        "terminal" => [0, 0, 0, 0, 0, 0, 1, 1, 1],
        "container" => [5; 9],
        "nuker" => [0, 0, 0, 0, 0, 0, 0, 0, 1],
        "factory" => [0, 0, 0, 0, 0, 0, 0, 1, 1],
        _ => [0; 9],
    };
    table[rcl.min(8) as usize]
}

/// 结构对指定资源的容量 (官方 capacityForResource 的结构部分)。
/// creep 见 `objects::Obj::capacity_for_resource`; 这里只覆盖非 creep 结构。
/// 官方 storeCapacityResource 形态的专用仓 (按资源分格计容量); 其余 (creep/container/storage/terminal/factory) 是通用仓
pub fn is_single_resource_store(obj_type: &str) -> bool {
    matches!(obj_type, "spawn" | "extension" | "tower" | "link" | "lab" | "powerSpawn" | "nuker")
}

pub fn structure_capacity_for_resource(structure_type: &str, resource: &str, energy_capacity: u32) -> u32 {
    match structure_type {
        "link" => LINK_CAPACITY,
        "container" => CONTAINER_CAPACITY,
        "storage" => STORAGE_CAPACITY,
        "terminal" => TERMINAL_CAPACITY,
        // 只存能量的结构: 非 energy 资源容量为 0
        // (lab / powerSpawn / nuker / terminal / factory 的多资源容量见 objects::Obj::capacity_for_resource)
        "spawn" | "extension" | "tower" => {
            if resource == "energy" {
                energy_capacity
            } else {
                0
            }
        }
        _ => energy_capacity,
    }
}

/// 建造完成后结构的能量容量 (官方 build.js 结构初始化表)
pub fn structure_energy_capacity(structure_type: &str, rcl: u8) -> u32 {
    match structure_type {
        "spawn" => SPAWN_ENERGY_CAPACITY,
        "extension" => extension_capacity(rcl),
        "storage" => STORAGE_CAPACITY,
        "container" => CONTAINER_CAPACITY,
        "tower" => TOWER_CAPACITY,
        "terminal" => TERMINAL_CAPACITY,
        "link" => LINK_CAPACITY,
        "lab" => LAB_ENERGY_CAPACITY,
        "powerSpawn" => POWER_SPAWN_ENERGY_CAPACITY,
        "nuker" => NUKER_ENERGY_CAPACITY,
        "factory" => FACTORY_CAPACITY,
        _ => 0,
    }
}

/// 阻挡移动的对象类型 (OBSTACLE_OBJECT_TYPES, 官方驱动常量)
pub const OBSTACLE_OBJECT_TYPES: &[&str] = &[    "spawn",
    "creep",
    "powerCreep",
    "source",
    "mineral",
    "deposit",
    "controller",
    "constructedWall",
    "extension",
    "link",
    "storage",
    "tower",
    "observer",
    "powerSpawn",
    "powerBank",
    "lab",
    "terminal",
    "nuker",
    "factory",
    "invaderCore",
];

/// 属于 `OwnedStructure` 的结构类型 (官方 API: 只有这些有 `owner`, 才带 `user`)。
/// 路 / 容器 / 墙 / 敌方对象不在其列 —— 它们不产生房间可见性 (官方 D/runtime/data.js:113-123)。
pub fn is_owned_structure(structure_type: &str) -> bool {
    matches!(
        structure_type,
        "spawn"
            | "extension"
            | "rampart"
            | "link"
            | "storage"
            | "tower"
            | "observer"
            | "powerSpawn"
            | "extractor"
            | "lab"
            | "terminal"
            | "nuker"
            | "factory"
            | "controller"
    )
}

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
/// link 容量 (官方 LINK_CAPACITY): creep→link 转移的容量来源
pub const LINK_CAPACITY: u32 = 800;
/// link 转移损耗 (官方 LINK_LOSS_RATIO, 本模拟器未实现 link.transferEnergy)
pub const LINK_LOSS_RATIO: f64 = 0.03;
/// 道路在墙上的造价/血量/衰减系数 (官方 CONSTRUCTION_COST_ROAD_WALL_RATIO)
pub const CONSTRUCTION_COST_ROAD_WALL_RATIO: u32 = 150;

/// 容器衰减: 每 CONTAINER_DECAY_TIME 掉 CONTAINER_DECAY 血 (自有房间 500t)
pub const CONTAINER_DECAY: u32 = 5000;
pub const CONTAINER_DECAY_TIME: u64 = 100;
pub const CONTAINER_DECAY_TIME_OWNED: u64 = 500;

/// rampart 衰减: 每 RAMPART_DECAY_TIME 掉 RAMPART_DECAY_AMOUNT 血
pub const RAMPART_DECAY_AMOUNT: u32 = 300;
pub const RAMPART_DECAY_TIME: u64 = 100;

/// rampart 血量上限按 RCL (官方 RAMPART_HITS_MAX)
pub fn rampart_hits_max(rcl: u8) -> u32 {
    match rcl {
        0 | 1 => 0,
        2 => 300_000,
        3 => 1_000_000,
        4 => 3_000_000,
        5 => 10_000_000,
        6 => 30_000_000,
        7 => 100_000_000,
        _ => 300_000_000,
    }
}

/// 废墟存续时间 (官方 RUIN_DECAY / RUIN_DECAY_STRUCTURES)
pub const RUIN_DECAY: u64 = 500;
pub fn ruin_decay(structure_type: &str) -> u64 {
    match structure_type {
        "powerBank" => 10,
        _ => RUIN_DECAY,
    }
}

/// 安全模式 (官方 SAFE_MODE_DURATION / SAFE_MODE_COOLDOWN / CONTROLLER_DOWNGRADE_SAFEMODE_THRESHOLD)
pub const SAFE_MODE_DURATION: u64 = 20_000;
pub const SAFE_MODE_COOLDOWN: u64 = 50_000;
pub const CONTROLLER_DOWNGRADE_SAFEMODE_THRESHOLD: u64 = 5_000;

/// 该结构类型是否受 CONTROLLER_STRUCTURES 数量上限约束 (官方表中有此键)
pub fn has_controller_structure_limit(structure_type: &str) -> bool {
    matches!(
        structure_type,
        "spawn"
            | "extension"
            | "road"
            | "constructedWall"
            | "rampart"
            | "link"
            | "storage"
            | "tower"
            | "observer"
            | "powerSpawn"
            | "extractor"
            | "lab"
            | "terminal"
            | "container"
            | "nuker"
            | "factory"
    )
}

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

/// GCL 到达 `level` 所需的累计分数: (level-1)^GCL_POW × GCL_MULTIPLY (官方 game.js:131 gclBaseProgress)。
pub fn gcl_base_progress(level: u32) -> u64 {
    (GCL_MULTIPLY as f64 * ((level.max(1) - 1) as f64).powf(GCL_POW)) as u64
}

/// GCL: 从 `level` 升到 `level+1` 所需的分数增量 (官方 game.js:162 progressTotal):
/// level^POW × MULT − (level−1)^POW × MULT。GCL1→2 = 1e6, 2→3 ≈ 4.278e6。
pub fn gcl_progress_total(level: u32) -> u64 {
    gcl_base_progress(level + 1) - gcl_base_progress(level)
}

/// 官方 game.js:130: level = floor((gcl / MULT)^(1/POW)) + 1
pub fn gcl_level_for(points: u64) -> u32 {
    let mut level = ((points as f64 / GCL_MULTIPLY as f64).powf(1.0 / GCL_POW)).floor() as u32 + 1;
    // 浮点边界修正: 保证 base(level) <= points < base(level+1)
    while level > 1 && gcl_base_progress(level) > points {
        level -= 1;
    }
    while gcl_base_progress(level + 1) <= points {
        level += 1;
    }
    level
}

// ── 工业: 矿物 / 实验室 / 终端 / 观察者 / PowerSpawn / 工厂 (官方 constants.js) ──

pub const HARVEST_MINERAL_POWER: u32 = 1;
pub const EXTRACTOR_COOLDOWN: u32 = 5;
pub const MINERAL_REGEN_TIME: u64 = 50_000;
/// 官方 MINERAL_DENSITY: 密度等级 → 再生后的储量
pub const MINERAL_DENSITY: &[(u8, u32)] = &[(1, 15_000), (2, 35_000), (3, 70_000), (4, 100_000)];
/// 官方 MINERAL_DENSITY_PROBABILITY (累积概率, 按键升序判定)
pub const MINERAL_DENSITY_PROBABILITY: &[(u8, f64)] = &[(1, 0.1), (2, 0.5), (3, 0.9), (4, 1.0)];
pub const MINERAL_DENSITY_CHANGE: f64 = 0.05;
pub const DENSITY_LOW: u8 = 1;
pub const DENSITY_MODERATE: u8 = 2;
pub const DENSITY_ULTRA: u8 = 4;

pub fn mineral_density_amount(density: u8) -> u32 {
    MINERAL_DENSITY
        .iter()
        .find(|(d, _)| *d == density)
        .map(|(_, a)| *a)
        .unwrap_or(0)
}

pub const LAB_MINERAL_CAPACITY: u32 = 3000;
pub const LAB_ENERGY_CAPACITY: u32 = 2000;
pub const LAB_BOOST_ENERGY: u32 = 20;
pub const LAB_BOOST_MINERAL: u32 = 30;
pub const LAB_REACTION_AMOUNT: u32 = 5;
pub const LAB_UNBOOST_ENERGY: u32 = 0;
pub const LAB_UNBOOST_MINERAL: u32 = 15;

pub const TERMINAL_COOLDOWN: u64 = 10;
pub const OBSERVER_RANGE: i64 = 10;

pub const POWER_SPAWN_ENERGY_CAPACITY: u32 = 5000;
pub const POWER_SPAWN_POWER_CAPACITY: u32 = 100;
pub const POWER_SPAWN_ENERGY_RATIO: u32 = 50;
/// GPL: level = floor((power / POWER_LEVEL_MULTIPLY)^(1/POWER_LEVEL_POW))
pub const POWER_LEVEL_MULTIPLY: f64 = 1000.0;
pub const POWER_LEVEL_POW: f64 = 2.0;

pub const FACTORY_CAPACITY: u32 = 50_000;
pub const NUKER_ENERGY_CAPACITY: u32 = 300_000;
pub const NUKER_GHODIUM_CAPACITY: u32 = 5000;

/// 官方 BOOSTS[part][boost][method] (无该加成返回 None)
pub fn boost_effect(part: &str, boost: &str, method: &str) -> Option<f64> {
    BOOSTS
        .iter()
        .find(|(p, b, m, _)| *p == part && *b == boost && *m == method)
        .map(|(_, _, _, v)| *v)
}

/// 官方 `C.BOOSTS[part][boost]` 是否存在 (boostCreep 选部件用)
pub fn boost_applies(part: &str, boost: &str) -> bool {
    BOOSTS.iter().any(|(p, b, _, _)| *p == part && *b == boost)
}

/// 官方 REACTIONS[a][b]
pub fn reaction_product(a: &str, b: &str) -> Option<&'static str> {
    REACTIONS.iter().find(|(x, y, _)| *x == a && *y == b).map(|(_, _, p)| *p)
}

/// 官方 utils.getReactionVariants: 生成 `compound` 的全部 (试剂1, 试剂2), 按官方迭代序
pub fn reaction_variants(compound: &str) -> Vec<(&'static str, &'static str)> {
    REACTIONS
        .iter()
        .filter(|(_, _, p)| *p == compound)
        .map(|(a, b, _)| (*a, *b))
        .collect()
}

pub fn reaction_time(product: &str) -> Option<u64> {
    REACTION_TIME.iter().find(|(p, _)| *p == product).map(|(_, t)| *t)
}

/// 官方 utils.calcTotalReactionsTime: 化合物整条反应链的总冷却
pub fn total_reactions_time(mineral: &str) -> u64 {
    let Some(t) = reaction_time(mineral) else { return 0 };
    // 官方 reagents 表: 每个产物取 REACTIONS 迭代中最后一次出现的 (试剂, 键) 组合
    let mut reagents: Option<(&str, &str)> = None;
    for (a, b, p) in REACTIONS {
        if *p == mineral {
            reagents = Some((b, a));
        }
    }
    match reagents {
        Some((r1, r2)) => t + total_reactions_time(r1) + total_reactions_time(r2),
        None => t,
    }
}

pub fn commodity(product: &str) -> Option<&'static Commodity> {
    COMMODITIES.iter().find(|c| c.product == product)
}

// ── 工业表 (由官方 common/lib/constants.js 逐字生成, 勿手改) ──

/// 官方 REACTIONS: (试剂1, 试剂2, 产物), 保持官方对象键的迭代顺序 (getReactionVariants 依赖此序)
pub const REACTIONS: &[(&str, &str, &str)] = &[
    ("H", "O", "OH"),
    ("H", "L", "LH"),
    ("H", "K", "KH"),
    ("H", "U", "UH"),
    ("H", "Z", "ZH"),
    ("H", "G", "GH"),
    ("O", "H", "OH"),
    ("O", "L", "LO"),
    ("O", "K", "KO"),
    ("O", "U", "UO"),
    ("O", "Z", "ZO"),
    ("O", "G", "GO"),
    ("Z", "K", "ZK"),
    ("Z", "H", "ZH"),
    ("Z", "O", "ZO"),
    ("L", "U", "UL"),
    ("L", "H", "LH"),
    ("L", "O", "LO"),
    ("K", "Z", "ZK"),
    ("K", "H", "KH"),
    ("K", "O", "KO"),
    ("G", "H", "GH"),
    ("G", "O", "GO"),
    ("U", "L", "UL"),
    ("U", "H", "UH"),
    ("U", "O", "UO"),
    ("OH", "UH", "UH2O"),
    ("OH", "UO", "UHO2"),
    ("OH", "ZH", "ZH2O"),
    ("OH", "ZO", "ZHO2"),
    ("OH", "KH", "KH2O"),
    ("OH", "KO", "KHO2"),
    ("OH", "LH", "LH2O"),
    ("OH", "LO", "LHO2"),
    ("OH", "GH", "GH2O"),
    ("OH", "GO", "GHO2"),
    ("X", "UH2O", "XUH2O"),
    ("X", "UHO2", "XUHO2"),
    ("X", "LH2O", "XLH2O"),
    ("X", "LHO2", "XLHO2"),
    ("X", "KH2O", "XKH2O"),
    ("X", "KHO2", "XKHO2"),
    ("X", "ZH2O", "XZH2O"),
    ("X", "ZHO2", "XZHO2"),
    ("X", "GH2O", "XGH2O"),
    ("X", "GHO2", "XGHO2"),
    ("ZK", "UL", "G"),
    ("UL", "ZK", "G"),
    ("LH", "OH", "LH2O"),
    ("ZH", "OH", "ZH2O"),
    ("GH", "OH", "GH2O"),
    ("KH", "OH", "KH2O"),
    ("UH", "OH", "UH2O"),
    ("LO", "OH", "LHO2"),
    ("ZO", "OH", "ZHO2"),
    ("KO", "OH", "KHO2"),
    ("UO", "OH", "UHO2"),
    ("GO", "OH", "GHO2"),
    ("LH2O", "X", "XLH2O"),
    ("KH2O", "X", "XKH2O"),
    ("ZH2O", "X", "XZH2O"),
    ("UH2O", "X", "XUH2O"),
    ("GH2O", "X", "XGH2O"),
    ("LHO2", "X", "XLHO2"),
    ("UHO2", "X", "XUHO2"),
    ("KHO2", "X", "XKHO2"),
    ("ZHO2", "X", "XZHO2"),
    ("GHO2", "X", "XGHO2"),
];

/// 官方 REACTION_TIME: 产物 → 反应冷却 tick
pub const REACTION_TIME: &[(&str, u64)] = &[
    ("OH", 20),
    ("ZK", 5),
    ("UL", 5),
    ("G", 5),
    ("UH", 10),
    ("UH2O", 5),
    ("XUH2O", 60),
    ("UO", 10),
    ("UHO2", 5),
    ("XUHO2", 60),
    ("KH", 10),
    ("KH2O", 5),
    ("XKH2O", 60),
    ("KO", 10),
    ("KHO2", 5),
    ("XKHO2", 60),
    ("LH", 15),
    ("LH2O", 10),
    ("XLH2O", 65),
    ("LO", 10),
    ("LHO2", 5),
    ("XLHO2", 60),
    ("ZH", 20),
    ("ZH2O", 40),
    ("XZH2O", 160),
    ("ZO", 10),
    ("ZHO2", 5),
    ("XZHO2", 60),
    ("GH", 10),
    ("GH2O", 15),
    ("XGH2O", 80),
    ("GO", 10),
    ("GHO2", 30),
    ("XGHO2", 150),
];

/// 官方 BOOSTS: (部件, 化合物, 方法, 倍率)
pub const BOOSTS: &[(&str, &str, &str, f64)] = &[
    ("work", "UO", "harvest", 3.0),
    ("work", "UHO2", "harvest", 5.0),
    ("work", "XUHO2", "harvest", 7.0),
    ("work", "LH", "build", 1.5),
    ("work", "LH", "repair", 1.5),
    ("work", "LH2O", "build", 1.8),
    ("work", "LH2O", "repair", 1.8),
    ("work", "XLH2O", "build", 2.0),
    ("work", "XLH2O", "repair", 2.0),
    ("work", "ZH", "dismantle", 2.0),
    ("work", "ZH2O", "dismantle", 3.0),
    ("work", "XZH2O", "dismantle", 4.0),
    ("work", "GH", "upgradeController", 1.5),
    ("work", "GH2O", "upgradeController", 1.8),
    ("work", "XGH2O", "upgradeController", 2.0),
    ("attack", "UH", "attack", 2.0),
    ("attack", "UH2O", "attack", 3.0),
    ("attack", "XUH2O", "attack", 4.0),
    ("ranged_attack", "KO", "rangedAttack", 2.0),
    ("ranged_attack", "KO", "rangedMassAttack", 2.0),
    ("ranged_attack", "KHO2", "rangedAttack", 3.0),
    ("ranged_attack", "KHO2", "rangedMassAttack", 3.0),
    ("ranged_attack", "XKHO2", "rangedAttack", 4.0),
    ("ranged_attack", "XKHO2", "rangedMassAttack", 4.0),
    ("heal", "LO", "heal", 2.0),
    ("heal", "LO", "rangedHeal", 2.0),
    ("heal", "LHO2", "heal", 3.0),
    ("heal", "LHO2", "rangedHeal", 3.0),
    ("heal", "XLHO2", "heal", 4.0),
    ("heal", "XLHO2", "rangedHeal", 4.0),
    ("carry", "KH", "capacity", 2.0),
    ("carry", "KH2O", "capacity", 3.0),
    ("carry", "XKH2O", "capacity", 4.0),
    ("move", "ZO", "fatigue", 2.0),
    ("move", "ZHO2", "fatigue", 3.0),
    ("move", "XZHO2", "fatigue", 4.0),
    ("tough", "GO", "damage", 0.7),
    ("tough", "GHO2", "damage", 0.5),
    ("tough", "XGHO2", "damage", 0.3),
];

/// 官方 COMMODITIES 的一条配方
pub struct Commodity {
    pub product: &'static str,
    /// 需要的工厂等级 (None = 无等级要求)
    pub level: Option<u8>,
    pub amount: u32,
    pub cooldown: u64,
    pub components: &'static [(&'static str, u32)],
}

/// 官方 COMMODITIES (工厂配方表)
pub const COMMODITIES: &[Commodity] = &[
    Commodity { product: "utrium_bar", level: None, amount: 100, cooldown: 20, components: &[("U", 500), ("energy", 200)] },
    Commodity { product: "U", level: None, amount: 500, cooldown: 20, components: &[("utrium_bar", 100), ("energy", 200)] },
    Commodity { product: "lemergium_bar", level: None, amount: 100, cooldown: 20, components: &[("L", 500), ("energy", 200)] },
    Commodity { product: "L", level: None, amount: 500, cooldown: 20, components: &[("lemergium_bar", 100), ("energy", 200)] },
    Commodity { product: "zynthium_bar", level: None, amount: 100, cooldown: 20, components: &[("Z", 500), ("energy", 200)] },
    Commodity { product: "Z", level: None, amount: 500, cooldown: 20, components: &[("zynthium_bar", 100), ("energy", 200)] },
    Commodity { product: "keanium_bar", level: None, amount: 100, cooldown: 20, components: &[("K", 500), ("energy", 200)] },
    Commodity { product: "K", level: None, amount: 500, cooldown: 20, components: &[("keanium_bar", 100), ("energy", 200)] },
    Commodity { product: "ghodium_melt", level: None, amount: 100, cooldown: 20, components: &[("G", 500), ("energy", 200)] },
    Commodity { product: "G", level: None, amount: 500, cooldown: 20, components: &[("ghodium_melt", 100), ("energy", 200)] },
    Commodity { product: "oxidant", level: None, amount: 100, cooldown: 20, components: &[("O", 500), ("energy", 200)] },
    Commodity { product: "O", level: None, amount: 500, cooldown: 20, components: &[("oxidant", 100), ("energy", 200)] },
    Commodity { product: "reductant", level: None, amount: 100, cooldown: 20, components: &[("H", 500), ("energy", 200)] },
    Commodity { product: "H", level: None, amount: 500, cooldown: 20, components: &[("reductant", 100), ("energy", 200)] },
    Commodity { product: "purifier", level: None, amount: 100, cooldown: 20, components: &[("X", 500), ("energy", 200)] },
    Commodity { product: "X", level: None, amount: 500, cooldown: 20, components: &[("purifier", 100), ("energy", 200)] },
    Commodity { product: "battery", level: None, amount: 50, cooldown: 10, components: &[("energy", 600)] },
    Commodity { product: "energy", level: None, amount: 500, cooldown: 10, components: &[("battery", 50)] },
    Commodity { product: "composite", level: Some(1), amount: 20, cooldown: 50, components: &[("utrium_bar", 20), ("zynthium_bar", 20), ("energy", 20)] },
    Commodity { product: "crystal", level: Some(2), amount: 6, cooldown: 21, components: &[("lemergium_bar", 6), ("keanium_bar", 6), ("purifier", 6), ("energy", 45)] },
    Commodity { product: "liquid", level: Some(3), amount: 12, cooldown: 60, components: &[("oxidant", 12), ("reductant", 12), ("ghodium_melt", 12), ("energy", 90)] },
    Commodity { product: "wire", level: None, amount: 20, cooldown: 8, components: &[("utrium_bar", 20), ("silicon", 100), ("energy", 40)] },
    Commodity { product: "switch", level: Some(1), amount: 5, cooldown: 70, components: &[("wire", 40), ("oxidant", 95), ("utrium_bar", 35), ("energy", 20)] },
    Commodity { product: "transistor", level: Some(2), amount: 1, cooldown: 59, components: &[("switch", 4), ("wire", 15), ("reductant", 85), ("energy", 8)] },
    Commodity { product: "microchip", level: Some(3), amount: 1, cooldown: 250, components: &[("transistor", 2), ("composite", 50), ("wire", 117), ("purifier", 25), ("energy", 16)] },
    Commodity { product: "circuit", level: Some(4), amount: 1, cooldown: 800, components: &[("microchip", 1), ("transistor", 5), ("switch", 4), ("oxidant", 115), ("energy", 32)] },
    Commodity { product: "device", level: Some(5), amount: 1, cooldown: 600, components: &[("circuit", 1), ("microchip", 3), ("crystal", 110), ("ghodium_melt", 150), ("energy", 64)] },
    Commodity { product: "cell", level: None, amount: 20, cooldown: 8, components: &[("lemergium_bar", 20), ("biomass", 100), ("energy", 40)] },
    Commodity { product: "phlegm", level: Some(1), amount: 2, cooldown: 35, components: &[("cell", 20), ("oxidant", 36), ("lemergium_bar", 16), ("energy", 8)] },
    Commodity { product: "tissue", level: Some(2), amount: 2, cooldown: 164, components: &[("phlegm", 10), ("cell", 10), ("reductant", 110), ("energy", 16)] },
    Commodity { product: "muscle", level: Some(3), amount: 1, cooldown: 250, components: &[("tissue", 3), ("phlegm", 3), ("zynthium_bar", 50), ("reductant", 50), ("energy", 16)] },
    Commodity { product: "organoid", level: Some(4), amount: 1, cooldown: 800, components: &[("muscle", 1), ("tissue", 5), ("purifier", 208), ("oxidant", 256), ("energy", 32)] },
    Commodity { product: "organism", level: Some(5), amount: 1, cooldown: 600, components: &[("organoid", 1), ("liquid", 150), ("tissue", 6), ("cell", 310), ("energy", 64)] },
    Commodity { product: "alloy", level: None, amount: 20, cooldown: 8, components: &[("zynthium_bar", 20), ("metal", 100), ("energy", 40)] },
    Commodity { product: "tube", level: Some(1), amount: 2, cooldown: 45, components: &[("alloy", 40), ("zynthium_bar", 16), ("energy", 8)] },
    Commodity { product: "fixtures", level: Some(2), amount: 1, cooldown: 115, components: &[("composite", 20), ("alloy", 41), ("oxidant", 161), ("energy", 8)] },
    Commodity { product: "frame", level: Some(3), amount: 1, cooldown: 125, components: &[("fixtures", 2), ("tube", 4), ("reductant", 330), ("zynthium_bar", 31), ("energy", 16)] },
    Commodity { product: "hydraulics", level: Some(4), amount: 1, cooldown: 800, components: &[("liquid", 150), ("fixtures", 3), ("tube", 15), ("purifier", 208), ("energy", 32)] },
    Commodity { product: "machine", level: Some(5), amount: 1, cooldown: 600, components: &[("hydraulics", 1), ("frame", 2), ("fixtures", 3), ("tube", 12), ("energy", 64)] },
    Commodity { product: "condensate", level: None, amount: 20, cooldown: 8, components: &[("keanium_bar", 20), ("mist", 100), ("energy", 40)] },
    Commodity { product: "concentrate", level: Some(1), amount: 3, cooldown: 41, components: &[("condensate", 30), ("keanium_bar", 15), ("reductant", 54), ("energy", 12)] },
    Commodity { product: "extract", level: Some(2), amount: 2, cooldown: 128, components: &[("concentrate", 10), ("condensate", 30), ("oxidant", 60), ("energy", 16)] },
    Commodity { product: "spirit", level: Some(3), amount: 1, cooldown: 200, components: &[("extract", 2), ("concentrate", 6), ("reductant", 90), ("purifier", 20), ("energy", 16)] },
    Commodity { product: "emanation", level: Some(4), amount: 1, cooldown: 800, components: &[("spirit", 2), ("extract", 2), ("concentrate", 3), ("keanium_bar", 112), ("energy", 32)] },
    Commodity { product: "essence", level: Some(5), amount: 1, cooldown: 600, components: &[("emanation", 1), ("spirit", 3), ("crystal", 110), ("ghodium_melt", 150), ("energy", 64)] },
];
