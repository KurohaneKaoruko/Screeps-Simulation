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

    /// 官方 `utils.checkStructureAgainstController` + `_calc_spawns`: 每类自有结构按到控制器的
    /// 切比雪夫距离由近到远占用 CONTROLLER_STRUCTURES 名额, 超出的标记 off (isActive() = false)。
    /// 距离相同按 id 序 (官方 roomObjects 键序) 先到先得; 全服上限为 1 的类型 (storage/terminal/
    /// extractor/observer/powerSpawn/nuker/factory) 只要当前 RCL 名额非 0 就全部激活 (官方特例)。
    /// 无主结构 (路/容器/墙) 恒为激活; 控制器不参与。
    pub fn refresh_structure_activity(&mut self) {
        let ctrl = self
            .controller()
            .filter(|c| c.mine && c.level > 0)
            .map(|c| (c.x as i32, c.y as i32, c.level));
        let mut types: Vec<String> = Vec::new();
        for o in self.objects.values() {
            if o.mine && o.obj_type != "controller" && constants::is_owned_structure(&o.obj_type) && !types.contains(&o.obj_type) {
                types.push(o.obj_type.clone());
            }
        }
        let mut off_ids: Vec<u64> = Vec::new();
        for t in &types {
            let ids: Vec<(u64, i32)> = self
                .objects
                .values()
                .filter(|o| o.mine && &o.obj_type == t)
                .map(|o| {
                    let dist = ctrl
                        .map(|(cx, cy, _)| (o.x as i32 - cx).abs().max((o.y as i32 - cy).abs()))
                        .unwrap_or(0);
                    (o.id, dist)
                })
                .collect();
            let Some((_, _, level)) = ctrl else {
                off_ids.extend(ids.iter().map(|(id, _)| *id));
                continue;
            };
            let allowed = constants::controller_structure_limit(t, level) as usize;
            if allowed == 0 {
                off_ids.extend(ids.iter().map(|(id, _)| *id));
                continue;
            }
            if constants::controller_structure_limit(t, 8) == 1 {
                continue;
            }
            let mut sorted = ids.clone();
            // 稳定排序: 距离升序, 同距离保持 id 序
            sorted.sort_by_key(|(_, d)| *d);
            off_ids.extend(sorted.iter().skip(allowed).map(|(id, _)| *id));
        }
        for o in self.objects.values_mut() {
            if o.mine && o.obj_type != "controller" && constants::is_owned_structure(&o.obj_type) {
                o.off = off_ids.contains(&o.id);
            }
        }
    }

    /// 我方单位/自有结构/工地在场 → 可见。
    /// 官方可见性来自带 `user` 的对象: 只有 creep、controller、OwnedStructure、自己的工地算数;
    /// 路 / 容器 / 墙 / 墓碑没有 user, 不能单独让房间可见。
    pub fn visible(&self) -> bool {
        self.objects.values().any(|o| {
            if !o.mine {
                return false;
            }
            match o.obj_type.as_str() {
                "creep" | "controller" | "constructionSite" => true,
                t => constants::is_owned_structure(t),
            }
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
    /// 账户累计 processPower 的 power (官方 users.power, GPL 由此推导)
    pub power_processed: u64,
    /// 引擎侧确定性随机数状态 (矿物密度变化等官方用 Math.random 的地方), 由场景 seed 播种
    pub rng_state: u64,
    /// 最近的终端转账记录 (新→旧, 最多 TRANSACTIONS_KEEP 条), 供 Game.market.*Transactions
    pub transactions: Vec<Transaction>,
}

/// 终端转账记录 (官方 transactions 集合的字段子集)
#[derive(Clone, Debug)]
pub struct Transaction {
    pub id: u64,
    pub time: u64,
    pub resource_type: String,
    pub amount: u32,
    pub from: String,
    pub to: String,
    pub description: Option<String>,
    /// 接收终端是否属于玩家 (单玩家模拟器: 自己发往自己的终端时出现在 incoming 与 outgoing 两侧)
    pub recipient_mine: bool,
}

/// 保留的最近转账条数 (官方 API 返回最近 100 条)
pub const TRANSACTIONS_KEEP: usize = 100;

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
            power_processed: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
            transactions: Vec::new(),
        }
    }

    /// 用场景 seed 播种引擎随机数
    pub fn seed_rng(&mut self, seed: u64) {
        self.rng_state = seed ^ 0x9E37_79B9_7F4A_7C15;
    }

    /// splitmix64 → [0, 1) 均匀分布 (替代官方处理器里的 Math.random, 保证可复现)
    pub fn random(&mut self) -> f64 {
        self.rng_state = self.rng_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// 官方 GPL (game.js): level = floor((power / 1000)^(1/2)), progress 相对本级起点
    pub fn gpl(&self) -> (u32, u64, u64) {
        let power = self.power_processed as f64;
        let level = (power / constants::POWER_LEVEL_MULTIPLY).powf(1.0 / constants::POWER_LEVEL_POW).floor();
        let base = level.powf(constants::POWER_LEVEL_POW) * constants::POWER_LEVEL_MULTIPLY;
        let total = (level + 1.0).powf(2.0) * 1000.0 - base;
        (level as u32, (power - base) as u64, total as u64)
    }

    /// 官方 driver.getWorldSize (与 shim 的 Game.map.getWorldSize 一致: 多房间 200, 单房间 50)
    pub fn world_size(&self) -> i64 {
        if self.rooms.len() > 1 {
            200
        } else {
            50
        }
    }

    /// 本 tick 被我方 observer 观察的房间 (官方 observeRoom: 下一 tick 对玩家可见)
    pub fn observed_rooms(&self) -> Vec<String> {
        let mut out = Vec::new();
        for room in self.rooms.values() {
            for o in room.objects.values() {
                if o.obj_type == "observer" && o.mine {
                    if let Some(r) = &o.observe_room {
                        if !out.contains(r) {
                            out.push(r.clone());
                        }
                    }
                }
            }
        }
        out
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
        if self.power_processed > 0 {
            mix_u64(&mut h, self.power_processed);
        }
        for (level, t) in &self.rcl_timeline {
            mix_u64(&mut h, *level as u64);
            mix_u64(&mut h, *t);
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
                    if let Some(b) = &part.boost {
                        mix_str(&mut h, b);
                    }
                }
                // 工业字段: 只在非默认值时混入 (旧场景的黄金哈希保持不变)
                if let Some(a) = obj.mineral_amount {
                    if obj.obj_type == "mineral" {
                        mix_u64(&mut h, a as u64);
                    }
                }
                if obj.cooldown_time > 0 {
                    mix_u64(&mut h, obj.cooldown_time);
                }
                if obj.pending_cooldown > 0 {
                    mix_u64(&mut h, obj.pending_cooldown as u64);
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
                "lab" | "powerSpawn" | "nuker" | "factory" => {
                    o.energy_capacity = constants::structure_energy_capacity(&o.obj_type, rcl);
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
        room.refresh_structure_activity();
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
    let room = world.rooms.get_mut(room_name).expect("room exists");
    obj.energy_capacity = constants::structure_energy_capacity(structure_type, room.rcl());
    room.objects.insert(id, obj);
    id
}

/// 空 Store 工具
pub fn energy_store(amount: u32) -> Store {
    let mut s = Store::new();
    s.insert("energy".into(), amount);
    s
}
