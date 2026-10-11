//! 世界与房间: 全部模拟状态的最终权威, 结算顺序对齐官方 processor.js。

use crate::engine::constants;
use crate::engine::objects::{self, Obj, Store};
use crate::engine::terrain::Terrain;
use std::collections::BTreeMap;

/// 房间事件日志条目 (官方 Room.getEventLog 的结构: {event, objectId, data})
#[derive(Clone, Debug)]
pub struct GameEvent {
    pub event: u32,
    pub object_id: u64,
    pub data: serde_json::Value,
}

pub struct Room {
    pub name: String,
    pub terrain: Terrain,
    pub objects: BTreeMap<u64, Obj>,
    /// 本 tick 发生的事件 (每 tick 开始时清空, 经快照传给 bot)
    pub events: Vec<GameEvent>,
}

impl Room {
    pub fn new(name: &str, terrain: Terrain) -> Room {
        Room {
            name: name.to_string(),
            terrain,
            objects: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// 记录一条事件 (引擎结算期间调用)
    pub fn push_event(&mut self, event: u32, object_id: u64, data: serde_json::Value) {
        self.events.push(GameEvent { event, object_id, data });
    }

    pub fn controller(&self) -> Option<&Obj> {
        self.objects.values().find(|o| o.obj_type == "controller")
    }

    pub fn controller_mut(&mut self) -> Option<&mut Obj> {
        self.objects
            .values_mut()
            .find(|o| o.obj_type == "controller")
    }

    pub fn rcl(&self) -> u8 {
        self.controller().map(|c| c.level).unwrap_or(0)
    }

    /// spawn + extension 的能量合计 (engine: _calc_spawns), 排除被停用 (off) 的结构
    pub fn energy_available(&self) -> u32 {
        self.objects
            .values()
            .filter(|o| o.mine && o.is_active() && (o.obj_type == "spawn" || o.obj_type == "extension"))
            .map(|o| o.store.get("energy").copied().unwrap_or(0))
            .sum()
    }

    pub fn energy_capacity_available(&self) -> u32 {
        self.objects
            .values()
            .filter(|o| o.mine && o.is_active() && (o.obj_type == "spawn" || o.obj_type == "extension"))
            .map(|o| o.energy_capacity)
            .sum()
    }

    /// 官方 _calc_spawns: 按到控制器的距离, 把超出 RCL 上限的 spawn/extension 标记为 off。
    /// 距离越远越先停用; 距离相同时按 id 序稳定。
    pub fn refresh_spawn_off(&mut self) {
        let rcl = self.rcl();
        let spawn_limit = constants::controller_structure_limit("spawn", rcl) as usize;
        let ext_limit = constants::controller_structure_limit("extension", rcl) as usize;
        let ctrl = self.controller().map(|c| (c.x as i32, c.y as i32));
        // (id, 是 spawn, 到控制器距离)
        let mut items: Vec<(u64, bool, i32)> = self
            .objects
            .values()
            .filter(|o| o.mine && (o.obj_type == "spawn" || o.obj_type == "extension"))
            .map(|o| {
                let dist = ctrl
                    .map(|(cx, cy)| {
                        (o.x as i32 - cx).abs().max((o.y as i32 - cy).abs())
                    })
                    .unwrap_or(0);
                (o.id, o.obj_type == "spawn", dist)
            })
            .collect();
        items.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        let mut spawn_left = spawn_limit;
        let mut ext_left = ext_limit;
        let mut off_ids: Vec<u64> = Vec::new();
        for (id, is_spawn, _) in items {
            let slot = if is_spawn { &mut spawn_left } else { &mut ext_left };
            if *slot > 0 {
                *slot -= 1;
            } else {
                off_ids.push(id);
            }
        }
        for o in self.objects.values_mut() {
            if o.obj_type == "spawn" || o.obj_type == "extension" {
                o.off = off_ids.contains(&o.id);
            }
        }
    }

    /// 我方单位或自有结构在场 → 可见
    pub fn visible(&self) -> bool {
        self.objects.values().any(|o| match o.obj_type.as_str() {
            "creep" => o.mine,
            "controller" => o.mine,
            other => o.mine && constants::construction_cost(other).is_some(),
        })
    }

    /// 建成结构 + 工地按 structureType 计数 (checkControllerAvailability)
    pub fn count_of_type(&self, structure_type: &str) -> usize {
        self.objects
            .values()
            .filter(|o| match o.obj_type.as_str() {
                "constructionSite" => {
                    o.structure_type.as_deref() == Some(structure_type)
                }
                other => other == structure_type,
            })
            .count()
    }

    /// 指定格是否有同类型结构 / 工地 / 阻挡结构 (checkConstructionSite 的对象部分)
    pub fn site_blocked(&self, structure_type: &str, x: u8, y: u8) -> bool {
        for o in self.objects.values() {
            if o.x != x || o.y != y {
                continue;
            }
            match o.obj_type.as_str() {
                "constructionSite" => return true,
                "rampart" | "road" => {}
                t if constants::construction_cost(t).is_some() && structure_type != "rampart" && structure_type != "road" => {
                    return true;
                }
                t if t == structure_type => return true,
                _ => {}
            }
        }
        false
    }

    /// extension 容量按当前 RCL 刷新 (RCL 变化时生效)
    pub fn refresh_extension_capacity(&mut self) {
        let cap = constants::extension_capacity(self.rcl());
        for o in self.objects.values_mut() {
            if o.obj_type == "extension" {
                o.energy_capacity = cap;
            }
        }
    }
}

pub struct World {
    pub tick: u64,
    pub rooms: BTreeMap<String, Room>,
    pub next_id: u64,
    /// GCL 累计升级量 (等级由阈值推导)
    pub gcl_progress: u64,
    /// 出生后累计孵化单位数 / 死亡数 (指标用)
    pub creeps_produced: u64,
    pub creeps_lost: u64,
    /// 能量统计: 采集 / 建造 / 升级 / 孵化 消耗
    pub energy_harvested: u64,
    pub energy_built: u64,
    pub energy_control: u64,
    pub energy_spawned: u64,
    /// 各 RCL 首次达到的 tick (1 基, 索引即等级)
    pub rcl_timeline: BTreeMap<u8, u64>,
}

impl World {
    pub fn new() -> World {
        World {
            tick: 0,
            rooms: BTreeMap::new(),
            next_id: 1,
            gcl_progress: 0,
            creeps_produced: 0,
            creeps_lost: 0,
            energy_harvested: 0,
            energy_built: 0,
            energy_control: 0,
            energy_spawned: 0,
            rcl_timeline: BTreeMap::new(),
        }
    }

    pub fn alloc_id(&mut self) -> u64 {
        alloc_id(&mut self.next_id)
    }

    /// 快照用 id 字符串 (对齐官方 24 位 hex 风格, 保证 JS 侧可用字符串比较)
    pub fn id_string(id: u64) -> String {
        format!("{id:024x}")
    }

    pub fn gcl_level(&self) -> u32 {
        let mut level = 1u32;
        let mut remaining = self.gcl_progress;
        loop {
            let need = constants::gcl_progress_total(level);
            if remaining >= need && level < 30 {
                remaining -= need;
                level += 1;
            } else {
                return level;
            }
        }
    }

    pub fn gcl_level_progress(&self) -> (u64, u64) {
        let level = self.gcl_level();
        let mut remaining = self.gcl_progress;
        for l in 1..level {
            remaining -= constants::gcl_progress_total(l);
        }
        (remaining, constants::gcl_progress_total(level))
    }

    /// 按 id 字符串查对象 (跨房间)
    pub fn get_object(&self, id: &str) -> Option<(&Room, &Obj)> {
        let numeric = u64::from_str_radix(id, 16).ok()?;
        for room in self.rooms.values() {
            if let Some(obj) = room.objects.get(&numeric) {
                return Some((room, obj));
            }
        }
        None
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// 字段级拆分借用: 持有 world.rooms 可变借用时仍可分配 id
pub fn alloc_id(next: &mut u64) -> u64 {
    let id = *next;
    *next += 1;
    id
}

// ── 场景加载后的初始结算 ──

/// 初始化 ageTime / 降级计时 / 结构初始能量等"第一次 tick 才会补全"的字段
pub fn prime_world(world: &mut World) {
    let tick = world.tick;
    for room in world.rooms.values_mut() {
        room.refresh_extension_capacity();
        let rcl = room.rcl();
        for o in room.objects.values_mut() {
            match o.obj_type.as_str() {
                "creep" => {
                    if !o.spawning {
                        let life = if o.body.iter().any(|p| p.part_type == "claim") {
                            constants::CREEP_CLAIM_LIFE_TIME
                        } else {
                            constants::CREEP_LIFE_TIME
                        };
                        o.age_time = Some(tick + life);
                    }
                }
                "controller" => {
                    if o.mine && o.downgrade_time.is_none() && o.level > 0 {
                        let dt = constants::CONTROLLER_DOWNGRADE
                            .iter()
                            .find(|(l, _)| *l == o.level)
                            .map(|(_, t)| *t)
                            .unwrap_or(20_000);
                        o.downgrade_time = Some(tick + dt + 1);
                    }
                    o.upgraded_this_tick = 0;
                }
                "road" | "container" => {
                    if o.next_decay_time.is_none() {
                        o.next_decay_time =
                            Some(tick + constants::ROAD_DECAY_TIME);
                    }
                    if o.obj_type == "container" && o.energy_capacity == 0 {
                        o.energy_capacity = constants::CONTAINER_CAPACITY;
                    }
                    if o.obj_type == "road" && o.hits == 0 {
                        o.hits = constants::ROAD_HITS;
                        o.hits_max = constants::ROAD_HITS;
                    }
                }
                "storage" => {
                    o.energy_capacity = constants::STORAGE_CAPACITY;
                }
                "terminal" => {
                    o.energy_capacity = constants::TERMINAL_CAPACITY;
                }
                "tower" => {
                    o.energy_capacity = constants::TOWER_CAPACITY;
                }
                "link" => {
                    o.energy_capacity = constants::LINK_CAPACITY;
                }
                _ => {}
            }
        }
        let _ = rcl;
    }
    // RCL 时间线起点
    for room in world.rooms.values() {
        if let Some(c) = room.controller() {
            if c.mine && c.level > 0 && !world.rcl_timeline.contains_key(&c.level) {
                world.rcl_timeline.insert(c.level, world.tick);
            }
        }
    }
    // 超 RCL 上限的 spawn/extension 停用 (官方 _calc_spawns)
    for room in world.rooms.values_mut() {
        room.refresh_spawn_off();
    }
}

/// 场景构造 helper: 放置结构并设定能量
pub fn place_structure(
    world: &mut World,
    room_name: &str,
    structure_type: &str,
    x: u8,
    y: u8,
    energy: u32,
    hits: Option<u32>,
) -> u64 {
    let id = world.alloc_id();
    let mut obj = objects::structure(id, structure_type, x, y, true);
    obj.store.insert("energy".into(), energy);
    if let Some(h) = hits {
        obj.hits = h;
    }
    match structure_type {
        "spawn" => obj.energy_capacity = constants::SPAWN_ENERGY_CAPACITY,
        "storage" => obj.energy_capacity = constants::STORAGE_CAPACITY,
        "container" => obj.energy_capacity = constants::CONTAINER_CAPACITY,
        "tower" => obj.energy_capacity = constants::TOWER_CAPACITY,
        "terminal" => obj.energy_capacity = constants::TERMINAL_CAPACITY,
        "link" => obj.energy_capacity = constants::LINK_CAPACITY,
        _ => {}
    }
    let room = world.rooms.get_mut(room_name).expect("room exists");
    room.objects.insert(id, obj);
    id
}

/// 空 Store 工具
pub fn energy_store(amount: u32) -> Store {
    let mut s = Store::new();
    s.insert("energy".into(), amount);
    s
}
