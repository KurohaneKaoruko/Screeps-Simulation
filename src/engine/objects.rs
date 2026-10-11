//! 世界对象模型: 与官方引擎 roomObjects 对齐的运行时形态。
//! 用 BTreeMap<u64, Obj> 保存, id 单调递增, 迭代顺序即官方引擎的对象处理顺序。

use crate::engine::constants;
use std::collections::BTreeMap;

pub type Store = BTreeMap<String, u32>;

pub fn store_sum(store: &Store) -> u32 {
    store.values().sum()
}

#[derive(Clone, Debug)]
pub struct BodyPart {
    pub part_type: String,
    pub hits: u32,
}

#[derive(Clone, Debug)]
pub struct Obj {
    pub id: u64,
    /// creep / source / controller / energy(掉落) / constructionSite / tombstone / 结构类型名
    pub obj_type: String,
    pub x: u8,
    pub y: u8,
    /// 结构/工地/控制器归属 (模拟器单玩家; source 等中立对象为 false)
    pub mine: bool,
    /// 通用资源仓储
    pub store: Store,
    /// energy 通道容量 (extension 按房间 RCL 动态刷新, 其余建对象时定)
    pub energy_capacity: u32,
    // ── creep ──
    pub name: Option<String>,
    pub body: Vec<BodyPart>,
    pub fatigue: u32,
    pub spawning: bool,
    pub age_time: Option<u64>,
    pub hits: u32,
    pub hits_max: u32,
    // ── 结构/工地/控制器 ──
    pub structure_type: Option<String>,
    pub progress: u32,
    pub progress_total: u32,
    pub level: u8,
    pub downgrade_time: Option<u64>,
    /// 本 tick 控制器已接收的升级量 (RCL8 限流)
    pub upgraded_this_tick: u32,
    // ── source ──
    pub next_regeneration_time: Option<u64>,
    // ── 衰减对象 ──
    pub next_decay_time: Option<u64>,
    /// 墓碑来源单位名
    pub creep_name: Option<String>,
    /// spawn 孵化任务 (仅 spawn 使用)
    pub spawn_job: Option<SpawnJob>,
    /// 超出 RCL 上限而被停用的 spawn/extension (官方 _calc_spawns 的 off 标记)
    pub off: bool,
    /// link 传输冷却 (官方 cooldown, 每 tick 减 1)
    pub cooldown: u32,
}

/// 孵化任务: name=目标单位名, need_time=总时长, spawn_time=完成 tick
#[derive(Clone, Debug)]
pub struct SpawnJob {
    pub name: String,
    pub need_time: u64,
    pub spawn_time: u64,
    /// 出生方向优先序 (官方 setDirections / opts.directions)
    pub directions: Option<Vec<i32>>,
}

impl Obj {
    pub fn live_parts(&self, part: &str) -> usize {
        self.body
            .iter()
            .filter(|p| p.part_type == part && p.hits > 0)
            .count()
    }

    /// WORK 有效功率 (部件 × 每件功率), 含 build/upgrade/harvest 用的活跃件数
    pub fn work_power(&self, per_part: u32) -> u32 {
        self.live_parts("work") as u32 * per_part
    }

    /// 是否阻挡移动 (不含 creep; creep 单独按占据规则处理)
    pub fn is_obstacle(&self) -> bool {
        match self.obj_type.as_str() {
            "creep" => false,
            "constructionSite" => self
                .structure_type
                .as_deref()
                .map(|t| constants::OBSTACLE_OBJECT_TYPES.contains(&t))
                .unwrap_or(false),
            "rampart" => false, // 自有 rampart 不阻挡; 模拟器无敌方 rampart
            other => constants::OBSTACLE_OBJECT_TYPES.contains(&other),
        }
    }

    pub fn store_capacity(&self) -> u32 {
        match self.obj_type.as_str() {
            "creep" => {
                self.body.iter().filter(|p| p.part_type == "carry").count() as u32
                    * constants::CARRY_CAPACITY
            }
            "tombstone" | "energy" => u32::MAX / 4,
            "link" => constants::LINK_CAPACITY,
            "container" => constants::CONTAINER_CAPACITY,
            "storage" => constants::STORAGE_CAPACITY,
            "terminal" => constants::TERMINAL_CAPACITY,
            _ => self.energy_capacity,
        }
    }

    /// 官方 `capacityForResource`: 该对象能容纳多少 `resource`。
    /// creep 是 CARRY×50, link 是 800, 只存能量的结构对非 energy 资源为 0。
    pub fn capacity_for_resource(&self, resource: &str) -> u32 {
        if self.obj_type == "creep" {
            return self.store_capacity();
        }
        constants::structure_capacity_for_resource(&self.obj_type, resource, self.energy_capacity)
    }

    /// 官方 `isActive()`: 超出 RCL 上限被停用的 spawn/extension 不参与能量结算
    pub fn is_active(&self) -> bool {
        !self.off
    }
}

// ── 工厂 ──

pub fn structure(id: u64, structure_type: &str, x: u8, y: u8, mine: bool) -> Obj {
    let hits = constants::structure_hits(structure_type);
    Obj {
        id,
        obj_type: structure_type.to_string(),
        x,
        y,
        mine,
        store: Store::default(),
        energy_capacity: 0,
        name: None,
        body: vec![],
        fatigue: 0,
        spawning: false,
        age_time: None,
        hits,
        hits_max: hits,
        structure_type: Some(structure_type.to_string()),
        progress: 0,
        progress_total: 0,
        level: 0,
        downgrade_time: None,
        upgraded_this_tick: 0,
        next_regeneration_time: None,
        next_decay_time: None,
        creep_name: None,
        spawn_job: None,
        off: false,
        cooldown: 0,
    }
}

pub fn source(id: u64, x: u8, y: u8, capacity: u32) -> Obj {
    let mut o = structure(id, "source", x, y, false);
    o.store.insert("energy".into(), capacity);
    o.energy_capacity = capacity;
    o.hits = 5000;
    o.hits_max = 5000;
    o
}

pub fn controller(id: u64, x: u8, y: u8, level: u8) -> Obj {
    let mut o = structure(id, "controller", x, y, true);
    o.level = level;
    o.hits = 1;
    o.hits_max = 1;
    o
}

pub fn creep_obj(id: u64, name: &str, x: u8, y: u8, body: Vec<BodyPart>) -> Obj {
    let hits = body.len() as u32 * 100;
    Obj {
        id,
        obj_type: "creep".into(),
        x,
        y,
        mine: true,
        store: Store::default(),
        energy_capacity: 0,
        name: Some(name.to_string()),
        body,
        fatigue: 0,
        spawning: false,
        age_time: None,
        hits,
        hits_max: hits,
        structure_type: None,
        progress: 0,
        progress_total: 0,
        level: 0,
        downgrade_time: None,
        upgraded_this_tick: 0,
        next_regeneration_time: None,
        next_decay_time: None,
        creep_name: None,
        spawn_job: None,
        off: false,
        cooldown: 0,
    }
}

pub fn dropped_energy(id: u64, x: u8, y: u8, amount: u32) -> Obj {
    let mut o = structure(id, "energy", x, y, false);
    o.store.insert("energy".into(), amount);
    o
}

pub fn construction_site(id: u64, structure_type: &str, x: u8, y: u8, progress_total: u32) -> Obj {
    let mut o = structure(id, "constructionSite", x, y, true);
    o.structure_type = Some(structure_type.to_string());
    o.progress_total = progress_total;
    o
}

pub fn tombstone(id: u64, x: u8, y: u8, creep_name: &str, decay_time: u64) -> Obj {
    let mut o = structure(id, "tombstone", x, y, true);
    o.creep_name = Some(creep_name.to_string());
    o.next_decay_time = Some(decay_time);
    o
}
