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
    /// 官方 body[i].boost: 实验室强化所用化合物 (None = 未强化)
    pub boost: Option<String>,
}

impl BodyPart {
    /// 满血、未强化的部件
    pub fn new(part_type: &str) -> BodyPart {
        BodyPart { part_type: part_type.to_string(), hits: 100, boost: None }
    }
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
    /// 敌方对象 (带 user 但非我方): rangedMassAttack / FIND_HOSTILE_* 的目标。
    /// 单玩家模拟器里只有场景可显式放置敌方对象。
    pub enemy: bool,
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
    // ── 战斗 (官方 creep `_damageToApply` / `_healToApply`, 在 tick 阶段统一应用) ──
    /// 本 tick 累积待结算伤害 (仅 creep 走累积路径; 结构即时扣血)
    pub damage_to_apply: u32,
    /// 本 tick 累积待结算治疗量
    pub heal_to_apply: u32,
    // ── 控制器保留 / 签名 / 攻击封锁 ──
    /// 保留到期 tick (None = 未被保留)
    pub reservation_end_time: Option<u64>,
    /// 控制器签名文本
    pub sign: Option<String>,
    /// 攻击封锁升级的到期 tick (官方 upgradeBlocked)
    pub upgrade_blocked: u64,
    // ── mineral ──
    pub mineral_type: Option<String>,
    pub mineral_amount: Option<u32>,
    /// 官方 mineral.density (DENSITY_LOW..DENSITY_ULTRA)
    pub density: u8,
    // ── 工业结构 ──
    /// 官方 cooldownTime: lab / terminal / factory 的冷却到期 tick (API cooldown = max(0, cooldownTime - time))
    pub cooldown_time: u64,
    /// 官方 extractor `_cooldown`: 本 tick 采矿后待写入的冷却 (tick 阶段先递减旧值再写入)
    pub pending_cooldown: u32,
    /// 官方 observer.observeRoom: 本 tick 观察的房间 (下一 tick 对 bot 可见, 再下一 tick 清空)
    pub observe_room: Option<String>,
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
            // 官方 _recalc-body: storeCapacity = calcBodyEffectiveness(CARRY, 'capacity', 50, withoutOldHits)
            // —— 存活 CARRY 部件 × 50 × boost 倍率 (KH ×2 / KH2O ×3 / XKH2O ×4)
            "creep" => self.calc_body_effectiveness("carry", "capacity", constants::CARRY_CAPACITY),
            "tombstone" | "energy" => u32::MAX / 4,
            "link" => constants::LINK_CAPACITY,
            "container" => constants::CONTAINER_CAPACITY,
            "storage" => constants::STORAGE_CAPACITY,
            // 官方 terminals/tick.js / factories/tick.js: 房间 RCL 不允许时 storeCapacity 为 0 (见 tick_store_capacity)
            "terminal" | "factory" => self.energy_capacity,
            "lab" => constants::LAB_ENERGY_CAPACITY + constants::LAB_MINERAL_CAPACITY,
            _ => self.energy_capacity,
        }
    }

    /// 官方 `storeCapacityResource` 存在 (专用格) 的对象: transfer 时按该资源单独计量,
    /// 否则按总量计量 (官方 transfer.js: `storeCapacityResource ? store[r] : calcResources`)
    pub fn has_resource_slots(&self) -> bool {
        matches!(
            self.obj_type.as_str(),
            "spawn" | "extension" | "tower" | "link" | "lab" | "powerSpawn" | "nuker"
        )
    }

    /// transfer 判满时与容量比较的已存量
    pub fn stored_for_capacity(&self, resource: &str) -> u32 {
        if self.has_resource_slots() {
            self.store.get(resource).copied().unwrap_or(0)
        } else {
            store_sum(&self.store)
        }
    }

    /// 官方 lab.mineralType: store 中第一个非能量且数量 > 0 的资源
    pub fn lab_mineral_type(&self) -> Option<&str> {
        self.store
            .iter()
            .find(|(k, v)| k.as_str() != "energy" && **v > 0)
            .map(|(k, _)| k.as_str())
    }

    /// 官方 lab.mineralAmount: store 总量 - 能量
    pub fn lab_mineral_amount(&self) -> u32 {
        store_sum(&self.store) - self.store.get("energy").copied().unwrap_or(0)
    }

    /// 掉落资源堆的资源类型 (每堆只有一种资源, 官方 `resourceType`)
    pub fn resource_type(&self) -> &str {
        self.store
            .keys()
            .next()
            .map(String::as_str)
            .unwrap_or("energy")
    }

    /// 掉落资源堆的数量 (官方 `object[resourceType]`)
    pub fn resource_amount(&self) -> u32 {
        self.store.values().next().copied().unwrap_or(0)
    }

    /// 官方 `capacityForResource`: 该对象能容纳多少 `resource`。
    /// creep 是 CARRY 容量, link 是 800, 只存能量的结构对非 energy 资源为 0;
    /// lab: 能量 2000, 矿物 3000 且只能是 lab 已持有的那一种 (官方 storeCapacityResource 随持有矿物切换)。
    pub fn capacity_for_resource(&self, resource: &str) -> u32 {
        match self.obj_type.as_str() {
            "creep" => self.store_capacity(),
            "lab" => {
                if resource == "energy" {
                    constants::LAB_ENERGY_CAPACITY
                } else {
                    match self.lab_mineral_type() {
                        Some(m) if m != resource => 0,
                        _ => constants::LAB_MINERAL_CAPACITY,
                    }
                }
            }
            "powerSpawn" => match resource {
                "energy" => constants::POWER_SPAWN_ENERGY_CAPACITY,
                "power" => constants::POWER_SPAWN_POWER_CAPACITY,
                _ => 0,
            },
            "nuker" => match resource {
                "energy" => constants::NUKER_ENERGY_CAPACITY,
                "G" => constants::NUKER_GHODIUM_CAPACITY,
                _ => 0,
            },
            "terminal" | "factory" => self.store_capacity(),
            t => constants::structure_capacity_for_resource(t, resource, self.energy_capacity),
        }
    }

    /// 官方 `isActive()`: 超出 RCL 上限被停用的结构 (见 Room::refresh_structure_activity)
    pub fn is_active(&self) -> bool {
        !self.off
    }

    /// 官方 `utils.calcBodyEffectiveness(body, part, method, basePower)`:
    /// 存活部件逐个累加 basePower × BOOSTS[part][boost][method] (无该 boost 加成按 1 倍)。
    /// 官方还会把 `_oldHits > 0` 的部件计入, 但伤害在 tick 末尾才应用, 意图阶段读到的
    /// 始终是 tick 开始的身体, 因此本模拟器用 `hits > 0` 即等价 (见 README 已知近似)。
    pub fn calc_body_effectiveness(&self, part: &str, method: &str, base_power: u32) -> u32 {
        let mut power = 0.0f64;
        for p in &self.body {
            if p.hits == 0 || p.part_type != part {
                continue;
            }
            let mut ipower = base_power as f64;
            if let Some(b) = &p.boost {
                if let Some(mult) = constants::boost_effect(part, b, method) {
                    ipower *= mult;
                }
            }
            power += ipower;
        }
        power as u32
    }

    /// 官方 build/repair/upgradeController 的 boost 加成: 每个强化 WORK 部件的额外量
    /// (`(倍率 - 1) × perPart`), 降序排列后只取前 `limit` 个 (官方 `boostedParts.slice(0, effect)`)。
    /// 注意官方不过滤受损部件 (map 全身体)。
    pub fn boosted_work_bonus(&self, method: &str, per_part: f64, limit: u32) -> f64 {
        let mut parts: Vec<f64> = self
            .body
            .iter()
            .map(|p| match (&p.boost, p.part_type.as_str()) {
                (Some(b), "work") => match constants::boost_effect("work", b, method) {
                    Some(m) if m > 0.0 => (m - 1.0) * per_part,
                    _ => 0.0,
                },
                _ => 0.0,
            })
            .collect();
        parts.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        parts.into_iter().take(limit as usize).sum()
    }

    /// 官方 creeps/tick.js `_applyDamage`: 有强化部件时, 伤害从身体头部起逐件按 TOUGH 的
    /// damage 倍率折减 (GO 0.7 / GHO2 0.5 / XGHO2 0.3), 返回实际扣血量。
    pub fn effective_damage(&self, damage: u32) -> u32 {
        if !self.body.iter().any(|p| p.boost.is_some()) {
            return damage;
        }
        let mut reduce = 0.0f64;
        let mut effective = damage as f64;
        for p in &self.body {
            if effective <= 0.0 {
                break;
            }
            let ratio = p
                .boost
                .as_deref()
                .and_then(|b| constants::boost_effect(&p.part_type, b, "damage"))
                .unwrap_or(1.0);
            let part_hits_effective = p.hits as f64 / ratio;
            let used = part_hits_effective.min(effective);
            reduce += used * (1.0 - ratio);
            effective -= used;
        }
        // 官方 damage -= Math.round(damageReduce) (JS round: .5 向上)
        let reduce = (reduce + 0.5).floor() as i64;
        (damage as i64 - reduce).max(0) as u32
    }

    /// 官方 `processor/intents/creeps/_recalc-body.js`: 由总 hits 反推各部件 hits。
    /// 从身体**末尾倒序**填充 (每件最多 100), 即伤害先摧毁列表尾部的部件。
    pub fn recalc_body(&mut self) {
        let mut hits = self.hits as i64;
        for part in self.body.iter_mut().rev() {
            // 注: 本仓库所用 rustc 1.95 (distro tarball 构建) 在 opt-level 3 下会把
            // `if hits > 100 { 100 } else { hits.max(0) as u32 }` 错误编译成不封顶的
            // `hits as u32` (debug/opt0 均正确)。clamp 写法经验证不受影响。
            part.hits = hits.clamp(0, 100) as u32;
            hits -= 100;
            if hits < 0 {
                hits = 0;
            }
        }
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
        enemy: false,
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
        damage_to_apply: 0,
        heal_to_apply: 0,
        reservation_end_time: None,
        sign: None,
        upgrade_blocked: 0,
        mineral_type: None,
        mineral_amount: None,
        density: 0,
        cooldown_time: 0,
        pending_cooldown: 0,
        observe_room: None,
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

pub fn mineral(id: u64, x: u8, y: u8, mineral_type: &str, amount: u32) -> Obj {
    let mut o = structure(id, "mineral", x, y, false);
    o.mineral_type = Some(mineral_type.to_string());
    o.mineral_amount = Some(amount);
    o.density = constants::DENSITY_MODERATE;
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
        enemy: false,
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
        damage_to_apply: 0,
        heal_to_apply: 0,
        reservation_end_time: None,
        sign: None,
        upgrade_blocked: 0,
        mineral_type: None,
        mineral_amount: None,
        density: 0,
        cooldown_time: 0,
        pending_cooldown: 0,
        observe_room: None,
    }
}

pub fn dropped_energy(id: u64, x: u8, y: u8, amount: u32) -> Obj {
    dropped_resource(id, x, y, "energy", amount)
}

/// 地上的资源堆 (官方 type 'energy' 对象, 每堆一种 resourceType)
pub fn dropped_resource(id: u64, x: u8, y: u8, resource: &str, amount: u32) -> Obj {
    let mut o = structure(id, "energy", x, y, false);
    o.store.insert(resource.to_string(), amount);
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
