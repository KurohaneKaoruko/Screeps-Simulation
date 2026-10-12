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
    /// terrain 的快照序列化缓存 (tick 期间地形不可变, 世界构建后刷新一次)
    pub terrain_json: String,
    pub objects: BTreeMap<u64, Obj>,
    /// 本 tick 发生的事件 (每 tick 开始时清空, 经快照传给 bot)
    pub events: Vec<GameEvent>,
}

impl Room {
    pub fn new(name: &str, terrain: Terrain) -> Room {
        let terrain_json = terrain.to_string();
        Room {
            name: name.to_string(),
            terrain,
            terrain_json,
            objects: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// 世界构建期地形定稿后刷新缓存 (terrain 在 tick 期间不可变)
    pub fn refresh_terrain_cache(&mut self) {
        self.terrain_json = self.terrain.to_string();
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

    /// 我方单位/自有结构在场 → 可见。
    /// 官方可见性来自带 `user` 的 creep、controller、OwnedStructure (driver 按这些对象下发房间);
    /// 工地不提供视野 (它只作为 userObjects 下发), 路 / 容器 / 墙 / 墓碑没有 user, 也不能让房间可见。
    pub fn visible(&self) -> bool {
        self.objects.values().any(|o| {
            if !o.mine {
                return false;
            }
            match o.obj_type.as_str() {
                "creep" | "controller" => true,
                t => constants::is_owned_structure(t),
            }
        })
    }

    /// 官方 utils.checkControllerAvailability: 只有自己的控制器按等级计, 否则 RCL 0;
    /// 数量统计房间内全部同类建筑 + 同类工地 (不分归属, 含本 tick 已建的工地)。
    pub fn controller_availability(&self, structure_type: &str) -> bool {
        let rcl = self
            .controller()
            .filter(|c| c.mine && c.level > 0)
            .map(|c| c.level)
            .unwrap_or(0);
        let count = self
            .objects
            .values()
            .filter(|o| {
                o.obj_type == structure_type
                    || (o.obj_type == "constructionSite" && o.structure_type.as_deref() == Some(structure_type))
            })
            .count();
        count < constants::controller_structure_limit(structure_type, rcl) as usize
    }

    /// 官方 utils.checkConstructionSite (对象分支 + 地形分支, 两者都要通过):
    /// - 同格已有同类型对象 / 任意工地 → 拒绝 (路上不能再建路, rampart 上不能再建 rampart)
    /// - extractor 必须建在没有 extractor 的矿上
    /// - 非路非 rampart 的建筑不能与任何可建造结构 (路/rampart 除外) 同格 (容器同样受限)
    /// - x/y 为 0 或 49 拒绝; 1/48 行列的非路/容器建筑要求外侧相邻 3 格都是墙
    /// - 墙上只能建路和 extractor
    pub fn construction_site_allowed(&self, structure_type: &str, x: u8, y: u8) -> bool {
        let at: Vec<&Obj> = self.objects.values().filter(|o| o.x == x && o.y == y).collect();
        if at.iter().any(|o| o.obj_type == structure_type) {
            return false;
        }
        if at.iter().any(|o| o.obj_type == "constructionSite") {
            return false;
        }
        let objects_ok = if structure_type == "extractor" {
            at.iter().any(|o| o.obj_type == "mineral") && !at.iter().any(|o| o.obj_type == "extractor")
        } else if structure_type != "rampart"
            && structure_type != "road"
            && at.iter().any(|o| {
                o.obj_type != "rampart" && o.obj_type != "road" && constants::construction_cost(&o.obj_type).is_some()
            })
        {
            false
        } else {
            !(x == 0 || y == 0 || x >= 49 || y >= 49)
        };
        if !objects_ok {
            return false;
        }
        // 地形分支
        if structure_type != "road" && structure_type != "container" && (x == 1 || x == 48 || y == 1 || y == 48) {
            let (x, y) = (x as i32, y as i32);
            let tiles: [(i32, i32); 3] = if x == 1 {
                [(0, y - 1), (0, y), (0, y + 1)]
            } else if x == 48 {
                [(49, y - 1), (49, y), (49, y + 1)]
            } else if y == 1 {
                [(x - 1, 0), (x, 0), (x + 1, 0)]
            } else {
                [(x - 1, 49), (x, 49), (x + 1, 49)]
            };
            if tiles.iter().any(|&(tx, ty)| {
                !(0..50).contains(&tx) || !(0..50).contains(&ty) || !self.terrain.is_wall(tx as usize, ty as usize)
            }) {
                return false;
            }
        }
        if structure_type == "extractor" {
            return true;
        }
        if structure_type != "road" && self.terrain.is_wall(x as usize, y as usize) {
            return false;
        }
        true
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

/// 旗 (官方 rooms.flags 的 `name~color~secondaryColor~x~y` 记录)。旗不是房间对象:
/// 不提供视野, 无视野房间里的旗照样出现在 Game.flags。
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub name: String,
    pub room: String,
    pub x: u8,
    pub y: u8,
    pub color: u8,
    pub secondary_color: u8,
}

pub struct World {
    pub tick: u64,
    pub rooms: BTreeMap<String, Room>,
    /// 全部旗 (按创建顺序; 名字全局唯一)
    pub flags: Vec<Flag>,
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
            flags: Vec::new(),
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
        constants::gcl_level_for(self.gcl_progress)
    }

    /// (本级进度, 本级总量), 官方 Game.gcl.progress / progressTotal
    pub fn gcl_level_progress(&self) -> (u64, u64) {
        let level = self.gcl_level();
        (
            self.gcl_progress - constants::gcl_base_progress(level),
            constants::gcl_progress_total(level),
        )
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

    /// 世界状态哈希 (FNV-1a 64)。用于黄金回归与确定性对拍:
    /// 任何结算顺序/数值/字段的变化都会改变哈希。刻意不使用 std 的 Hasher
    /// (其跨 Rust 版本不稳定)。覆盖: 全部对象的核心字段 + 房间能量 + 全局计数器。
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        fn mix(h: &mut u64, bytes: &[u8]) {
            for &b in bytes {
                *h ^= b as u64;
                *h = h.wrapping_mul(0x100000001b3);
            }
        }
        fn mix_str(h: &mut u64, s: &str) {
            mix(h, s.as_bytes());
            mix(h, b"\x1f");
        }
        fn mix_u64(h: &mut u64, v: u64) {
            mix(h, &v.to_le_bytes());
        }
        mix_u64(&mut h, self.tick);
        mix_u64(&mut h, self.next_id);
        mix_u64(&mut h, self.gcl_progress);
        mix_u64(&mut h, self.creeps_produced);
        mix_u64(&mut h, self.creeps_lost);
        mix_u64(&mut h, self.energy_harvested);
        mix_u64(&mut h, self.energy_built);
        mix_u64(&mut h, self.energy_control);
        mix_u64(&mut h, self.energy_spawned);
        for (level, t) in &self.rcl_timeline {
            mix_u64(&mut h, *level as u64);
            mix_u64(&mut h, *t);
        }
        // 旗只在存在时参与哈希 (不改变无旗场景的黄金基线)
        for f in &self.flags {
            mix_str(&mut h, &f.name);
            mix_str(&mut h, &f.room);
            mix(&mut h, &[f.x, f.y, f.color, f.secondary_color]);
        }
        for (name, room) in &self.rooms {
            mix_str(&mut h, name);
            mix_str(&mut h, &room.terrain_json);
            mix_u64(&mut h, room.energy_available() as u64);
            for obj in room.objects.values() {
                mix_u64(&mut h, obj.id);
                mix_str(&mut h, &obj.obj_type);
                mix_u64(&mut h, obj.x as u64);
                mix_u64(&mut h, obj.y as u64);
                mix_u64(&mut h, obj.hits as u64);
                mix_u64(&mut h, obj.mine as u64);
                mix_u64(&mut h, obj.enemy as u64);
                mix_u64(&mut h, obj.off as u64);
                mix_u64(&mut h, obj.fatigue as u64);
                mix_u64(&mut h, obj.spawning as u64);
                mix_u64(&mut h, obj.age_time.unwrap_or(0));
                mix_u64(&mut h, obj.progress as u64);
                mix_u64(&mut h, obj.level as u64);
                mix_u64(&mut h, obj.cooldown as u64);
                mix_u64(&mut h, obj.next_regeneration_time.unwrap_or(0));
                mix_u64(&mut h, obj.next_decay_time.unwrap_or(0));
                mix_u64(&mut h, obj.damage_to_apply as u64);
                mix_u64(&mut h, obj.heal_to_apply as u64);
                if let Some(n) = &obj.name {
                    mix_str(&mut h, n);
                }
                for (res, amount) in &obj.store {
                    mix_str(&mut h, res);
                    mix_u64(&mut h, *amount as u64);
                }
                for part in &obj.body {
                    mix_str(&mut h, &part.part_type);
                    mix_u64(&mut h, part.hits as u64);
                }
            }
        }
        h
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
                "road" | "container" | "rampart" => {
                    if o.next_decay_time.is_none() {
                        let interval = match o.obj_type.as_str() {
                            "road" => constants::ROAD_DECAY_TIME,
                            "rampart" => constants::RAMPART_DECAY_TIME,
                            _ => constants::CONTAINER_DECAY_TIME,
                        };
                        o.next_decay_time = Some(tick + interval);
                    }
                    if o.obj_type == "container" && o.energy_capacity == 0 {
                        o.energy_capacity = constants::CONTAINER_CAPACITY;
                    }
                    if o.obj_type == "road" && o.hits == 0 {
                        o.hits = constants::ROAD_HITS;
                        o.hits_max = constants::ROAD_HITS;
                    }
                    if o.obj_type == "rampart" {
                        o.hits_max = constants::rampart_hits_max(rcl);
                        if o.hits == 0 {
                            o.hits = 1;
                        }
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
    let mut obj = objects::structure(id, structure_type, x, y, constants::is_owned_structure(structure_type));
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
