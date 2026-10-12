//! Tick 推进: 对齐官方 processor.js 的阶段顺序。
//! 意图结算 → 移动仲裁 → 逐对象 tick (掉落衰减/Source 再生/creep 位移与寿命/
//! spawn 出生与自回能/控制器降级/道路容器衰减/墓碑消散) → 时间前进。

use crate::engine::constants;
use crate::engine::intents::{self, IntentList, TickEffects};
use crate::engine::movement::{self, Movement};
use crate::engine::objects;
use crate::engine::terrain::ROOM_SIZE;
use crate::engine::world::World;
use serde_json::json;

pub struct TickOutcome {
    pub creep_deaths: Vec<String>,
    /// (提交的移动意图数, 实际位移数)
    pub move_stats: (usize, usize),
}

pub fn run_tick(world: &mut World, movement: &mut Movement, intents: &IntentList) -> TickOutcome {
    let tick = world.tick;

    // 每 tick 重置: 控制器升级计数 / extension 容量 / 事件日志 / spawn 停用标记
    for room in world.rooms.values_mut() {
        room.refresh_extension_capacity();
        room.refresh_spawn_off();
        room.events.clear();
        for o in room.objects.values_mut() {
            if o.obj_type == "controller" {
                o.upgraded_this_tick = 0;
            }
        }
    }

    // 1) 意图结算 (含 move 登记)
    let effects = intents::apply_intents(world, intents);
    let mut movement = std::mem::take(movement);
    movement.init();
    for (room_name, id, dx, dy, old_fatigue) in &effects.moves {
        if let Some(room) = world.rooms.get(room_name) {
            if let Some(obj) = room.objects.get(id) {
                let nx = (obj.x as i32 + dx) as u8;
                let ny = (obj.y as i32 + dy) as u8;
                movement.add(room_name, *id, nx, ny, *old_fatigue);
            }
        }
    }

    for (room_name, puller, target) in &effects.pulls {
        if let Some(room) = world.rooms.get(room_name) {
            movement.add_pulling(room, *puller, *target);
        }
    }

    // 2) 每房间移动仲裁 (官方 processor.js:225,324: 每房间各自 movement.check)
    for (room_name, room) in &world.rooms {
        movement.check(room_name, room);
    }

    // 3) 逐对象 tick (id 序)
    let mut deaths: Vec<String> = Vec::new();
    let mut moved_count = 0usize;
    apply_suicides(world, &effects);
    let mut pending_crossings: Vec<(String, u64)> = Vec::new();

    let room_names: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names {
        // 先按类型分层处理 (engine 逐对象 tick, 对象类型互不交叠, 分层等价且借用友好)
        tick_dropped_energy(world, room_name);
        tick_sources(world, room_name);
        moved_count += tick_creeps(world, room_name, &mut movement, &mut deaths, &mut pending_crossings);
        tick_spawns(world, room_name, &mut movement);
        tick_controllers(world, room_name);
        tick_decay_structures(world, room_name);
        tick_cooldowns(world, room_name);
        tick_tombstones(world, room_name);
    }
    apply_crossings(world, &mut pending_crossings);

    let _ = tick;
    world.tick += 1;
    TickOutcome { creep_deaths: deaths, move_stats: (effects.moves.len(), moved_count) }
}

fn apply_suicides(world: &mut World, effects: &TickEffects) {
    for (room_name, id) in &effects.deaths {
        let removed = world
            .rooms
            .get_mut(room_name)
            .and_then(|r| r.objects.remove(id));
        if let Some(obj) = removed {
            world.creeps_lost += 1;
            intents::bury_creep(world, room_name, obj, constants::CREEP_CORPSE_RATE);
        }
    }
}

/// 掉落能量: 每 tick ceil(amount / 1000) 衰减
fn tick_dropped_energy(world: &mut World, room_name: &str) {
    let ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "energy")
        .map(|o| o.id)
        .collect();
    for id in ids {
        let room = world.rooms.get_mut(room_name).unwrap();
        if let Some(obj) = room.objects.get_mut(&id) {
            let amount = obj.store.get("energy").copied().unwrap_or(0);
            let decay = (amount + constants::ENERGY_DECAY - 1) / constants::ENERGY_DECAY;
            let left = amount.saturating_sub(decay);
            if left == 0 {
                room.objects.remove(&id);
            } else {
                obj.store.insert("energy".into(), left);
            }
        }
    }
}

/// Source 再生: 一旦低于容量即挂 300t 计时, 到点整段回满 (不是"清空后计时")
fn tick_sources(world: &mut World, room_name: &str) {
    // 官方 sources/tick.js: 有主或被保留 → 3000; 中立 → 1500 (能量截到 1500); 没有控制器 (SK 房) → 4000
    let controller_state = world.rooms[room_name]
        .controller()
        .map(|c| c.mine || c.enemy || c.reservation_end_time.is_some());
    let tick = world.tick;
    let room = world.rooms.get_mut(room_name).unwrap();
    for obj in room.objects.values_mut() {
        if obj.obj_type != "source" {
            continue;
        }
        let capacity = match controller_state {
            Some(true) => constants::SOURCE_ENERGY_CAPACITY,
            Some(false) => constants::SOURCE_ENERGY_NEUTRAL_CAPACITY,
            None => constants::SOURCE_ENERGY_KEEPER_CAPACITY,
        };
        if capacity == constants::SOURCE_ENERGY_NEUTRAL_CAPACITY && obj.energy_capacity != capacity {
            let e = obj.store.get("energy").copied().unwrap_or(0).min(capacity);
            obj.store.insert("energy".into(), e);
        }
        obj.energy_capacity = capacity;
        let energy = obj.store.get("energy").copied().unwrap_or(0);
        if energy < capacity {
            if obj.next_regeneration_time.is_none() {
                obj.next_regeneration_time = Some(tick + constants::ENERGY_REGEN_TIME);
            }
            if tick >= obj.next_regeneration_time.unwrap_or(0).saturating_sub(1) {
                obj.store.insert("energy".into(), capacity);
                obj.next_regeneration_time = None;
            }
        } else {
            obj.next_regeneration_time = None;
        }
    }
}

/// creep: 位移应用 → 边缘越房 → 寿命 → 疲劳恢复
fn tick_creeps(
    world: &mut World,
    room_name: &str,
    movement: &mut Movement,
    deaths: &mut Vec<String>,
    crossings: &mut Vec<(String, u64)>,
) -> usize {
    let ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "creep" && !o.spawning)
        .map(|o| o.id)
        .collect();
    let tick = world.tick;
    let mut moved_count = 0usize;
    for id in ids {
        // 位移
        if movement.execute(world.rooms.get_mut(room_name).unwrap(), id) {
            moved_count += 1;
        }
        let room = world.rooms.get_mut(room_name).unwrap();
        let Some(obj) = room.objects.get_mut(&id) else { continue };

        // 边缘格: 立即准备越房 (官方 interRoom; 无法停留于出口格)
        let at_edge = obj.x == 0 || obj.x == 49 || obj.y == 0 || obj.y == 49;
        if at_edge {
            crossings.push((room_name.to_string(), id));
        }

        // 寿命
        if obj.age_time.is_none() {
            let life = if obj.body.iter().any(|p| p.part_type == "claim") {
                constants::CREEP_CLAIM_LIFE_TIME
            } else {
                constants::CREEP_LIFE_TIME
            };
            obj.age_time = Some(tick + life);
        }
        if tick >= obj.age_time.unwrap_or(u64::MAX).saturating_sub(1) {
            let name = obj.name.clone().unwrap_or_default();
            let dead = world
                .rooms
                .get_mut(room_name)
                .unwrap()
                .objects
                .remove(&id)
                .unwrap();
            world.creeps_lost += 1;
            deaths.push(name);
            intents::bury_creep(world, room_name, dead, constants::CREEP_CORPSE_RATE);
            continue;
        }

        // 疲劳恢复 (官方 creeps/tick.js: -2 × MOVE, 经 _add-fatigue 先抵自己、剩余交给拉人者)
        let moves = obj.live_parts("move") as i64;
        if moves > 0 {
            movement.add_fatigue(room, id, -2 * moves);
        }
        let Some(obj) = room.objects.get_mut(&id) else { continue };

        // 伤害/治疗统一应用 (官方 processor/intents/creeps/tick.js: 在 creep tick 末尾)
        let old_hits = obj.hits;
        if obj.damage_to_apply > 0 {
            obj.hits = obj.hits.saturating_sub(obj.damage_to_apply);
            obj.damage_to_apply = 0;
        }
        if obj.heal_to_apply > 0 {
            obj.hits = (obj.hits + obj.heal_to_apply).min(obj.hits_max);
            obj.heal_to_apply = 0;
        }
        if obj.hits == 0 {
            // 战斗死亡: 与自然死亡同样掉落 (官方 _die)
            let name = obj.name.clone().unwrap_or_default();
            let dead = world
                .rooms
                .get_mut(room_name)
                .unwrap()
                .objects
                .remove(&id)
                .unwrap();
            world.creeps_lost += 1;
            deaths.push(name);
            intents::bury_creep(world, room_name, dead, constants::CREEP_CORPSE_RATE);
            continue;
        }
        if obj.hits != old_hits {
            // 官方 _recalc-body: 由总 hits 反推部件; 容量缩小则掉落超容资源
            obj.recalc_body();
            if obj.hits < old_hits {
                intents::drop_resources_without_space(world, room_name, id);
            }
        }
    }
    // 孵化中的 creep: 若对应 spawn 消失则一并移除
    let spawning_creeps: Vec<(u64, String)> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "creep" && o.spawning)
        .map(|o| (o.id, o.name.clone().unwrap_or_default()))
        .collect();
    for (id, name) in spawning_creeps {
        let has_spawn = world.rooms[room_name].objects.values().any(|s| {
            s.obj_type == "spawn" && s.spawn_job.as_ref().map(|j| j.name == name).unwrap_or(false)
        });
        if !has_spawn {
            world.rooms.get_mut(room_name).unwrap().objects.remove(&id);
        }
    }
    moved_count
}

/// spawn: 出生结算 + 能量自动回复 (energyAvailable<300 时每 tick +1)
fn tick_spawns(world: &mut World, room_name: &str, movement: &mut Movement) {
    let spawn_ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "spawn")
        .map(|o| o.id)
        .collect();
    let tick = world.tick;
    for spawn_id in spawn_ids {
        // 出生
        let job = world.rooms[room_name].objects[&spawn_id].spawn_job.clone();
        if let Some(job) = job {
            if tick >= job.spawn_time.saturating_sub(1) {
                let birth = {
                    let room = &world.rooms[room_name];
                    let spawn = &room.objects[&spawn_id];
                    movement::find_birth_tile(room, spawn, movement, job.directions.as_deref())
                };
                let room = world.rooms.get_mut(room_name).unwrap();
                if let Some((x, y)) = birth {
                    // 找到孵化中的 creep 并落位
                    let creep_id = room
                        .objects
                        .values()
                        .find(|o| o.obj_type == "creep" && o.spawning && o.name.as_deref() == Some(job.name.as_str()))
                        .map(|o| o.id);
                    if let Some(cid) = creep_id {
                        let creep = room.objects.get_mut(&cid).unwrap();
                        creep.x = x;
                        creep.y = y;
                        creep.spawning = false;
                        creep.age_time =
                            Some(tick + if creep.body.iter().any(|p| p.part_type == "claim") {
                                constants::CREEP_CLAIM_LIFE_TIME
                            } else {
                                constants::CREEP_LIFE_TIME
                            });
                    }
                    room.objects.get_mut(&spawn_id).unwrap().spawn_job = None;
                } else {
                    // 周围全堵: 顺延 1t (官方行为)
                    let spawn = room.objects.get_mut(&spawn_id).unwrap();
                    if let Some(j) = spawn.spawn_job.as_mut() {
                        j.spawn_time = tick + 1;
                    }
                }
            }
        }
        // 自动能量回复 (被停用的 spawn 不回能, 官方 spawns/tick.js:35-42)
        let room = world.rooms.get_mut(room_name).unwrap();
        let energy_available = room.energy_available();
        let spawn = room.objects.get_mut(&spawn_id).unwrap();
        if spawn.mine
            && !spawn.off
            && energy_available < constants::SPAWN_ENERGY_CAPACITY
            && spawn.store.get("energy").copied().unwrap_or(0) < constants::SPAWN_ENERGY_CAPACITY
        {
            let energy = spawn.store.get("energy").copied().unwrap_or(0);
            spawn.store.insert("energy".into(), energy + 1);
        }
    }
}

/// 控制器: 降级计时初始化 / 升级后恢复 / 超时降级
fn tick_controllers(world: &mut World, room_name: &str) {
    let tick = world.tick;
    let Some(controller_id) = world.rooms[room_name]
        .objects
        .values()
        .find(|o| o.obj_type == "controller")
        .map(|o| o.id)
    else {
        return;
    };
    let controller = world.rooms.get_mut(room_name).unwrap().objects.get_mut(&controller_id).unwrap();
    // 官方 controllers/tick.js:10: 保留到期 (gameTime >= endTime-1) 或房间已有主人 → 清除保留
    if let Some(end) = controller.reservation_end_time {
        if tick + 1 >= end || controller.mine || controller.enemy {
            controller.reservation_end_time = None;
        }
    }
    if !controller.mine {
        return;
    }
    let level = controller.level;
    if controller.downgrade_time.is_none() {
        controller.downgrade_time = Some(tick + downgrade(level) + 1);
        return;
    }
    if controller.upgraded_this_tick > 0 {
        let dt = controller.downgrade_time.unwrap_or(tick);
        controller.downgrade_time = Some(
            (dt + constants::CONTROLLER_DOWNGRADE_RESTORE + 1).min(tick + downgrade(level) + 1),
        );
        return;
    }
    while tick >= controller.downgrade_time.unwrap_or(u64::MAX).saturating_sub(1) && controller.level > 0 {
        controller.level -= 1;
        let new_level = controller.level;
        if new_level == 0 {
            controller.progress = 0;
            controller.mine = false;
            controller.downgrade_time = None;
        } else {
            controller.downgrade_time =
                Some(controller.downgrade_time.unwrap_or(tick) + downgrade(new_level) / 2 + 1);
            // 降级后进度 += 新等级需求的 90% (官方: progress += round(LEVELS[level]*0.9))
            let keep = ((controller_levels(new_level) as f64) * 0.9).round() as u32;
            controller.progress += keep;
        }
    }
}

fn downgrade(level: u8) -> u64 {
    constants::CONTROLLER_DOWNGRADE
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, t)| *t)
        .unwrap_or(20_000)
}

fn controller_levels(level: u8) -> u32 {
    constants::CONTROLLER_LEVELS
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, p)| *p)
        .unwrap_or(u32::MAX)
}

/// 道路/容器/rampart 衰减
fn tick_decay_structures(world: &mut World, room_name: &str) {
    let tick = world.tick;
    let owned = world.rooms[room_name]
        .controller()
        .map(|c| c.mine && c.level > 0)
        .unwrap_or(false);
    let rcl = world.rooms[room_name].rcl();
    let ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "road" || o.obj_type == "container" || o.obj_type == "rampart")
        .map(|o| o.id)
        .collect();
    for id in ids {
        let room = world.rooms.get_mut(room_name).unwrap();
        let Some(obj) = room.objects.get_mut(&id) else { continue };
        let due = obj
            .next_decay_time
            .map(|t| tick >= t.saturating_sub(1))
            .unwrap_or(true);
        if !due {
            continue;
        }
        let decay_amount = match obj.obj_type.as_str() {
            "road" => {
                // 官方 roads/tick.js:16-19: 沼泽 ×5, 墙 ×150
                let mut amount = constants::ROAD_DECAY_AMOUNT as u32;
                if room.terrain.is_wall(obj.x as usize, obj.y as usize) {
                    amount *= constants::CONSTRUCTION_COST_ROAD_WALL_RATIO;
                } else if room.terrain.is_swamp(obj.x as usize, obj.y as usize) {
                    amount *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
                }
                amount
            }
            "rampart" => {
                // 官方 ramparts/tick.js:19-40: 每 100t 减 300, hitsMax 按 RCL
                obj.hits_max = constants::rampart_hits_max(rcl);
                constants::RAMPART_DECAY_AMOUNT
            }
            _ => constants::CONTAINER_DECAY,
        };
        obj.hits = obj.hits.saturating_sub(decay_amount);
        if obj.hits == 0 {
            let store = obj.store.clone();
            let (x, y) = (obj.x, obj.y);
            let type_name = obj.obj_type.clone();
            room.objects.remove(&id);
            room.push_event(intents::EVENT_OBJECT_DESTROYED, id, json!({ "type": type_name }));
            for (resource, amount) in store {
                if amount > 0 {
                    intents::create_dropped(world, room_name, x, y, &resource, amount);
                }
            }
        } else {
            let interval = match obj.obj_type.as_str() {
                "road" => constants::ROAD_DECAY_TIME,
                "rampart" => constants::RAMPART_DECAY_TIME,
                _ if owned => constants::CONTAINER_DECAY_TIME_OWNED,
                _ => constants::CONTAINER_DECAY_TIME,
            };
            obj.next_decay_time = Some(tick + interval);
        }
    }
}

/// link 冷却: 每 tick 减 1 (官方 links/tick.js, 在意图结算之后执行)
fn tick_cooldowns(world: &mut World, room_name: &str) {
    let room = world.rooms.get_mut(room_name).unwrap();
    for o in room.objects.values_mut() {
        if o.cooldown > 0 {
            o.cooldown -= 1;
        }
    }
}

/// 墓碑消散: 内含资源散落原地
fn tick_tombstones(world: &mut World, room_name: &str) {
    let tick = world.tick;
    let ids: Vec<(u64, u8, u8, objects::Store)> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| {
            o.obj_type == "tombstone" && o.next_decay_time.map(|t| tick >= t.saturating_sub(1)).unwrap_or(true)
        })
        .map(|o| (o.id, o.x, o.y, o.store.clone()))
        .collect();
    for (id, x, y, store) in ids {
        world.rooms.get_mut(room_name).unwrap().objects.remove(&id);
        for (resource, amount) in store {
            if amount > 0 {
                intents::create_dropped(world, room_name, x, y, &resource, amount);
            }
        }
    }
}

/// 边缘越房: 出口格上的 creep 立即镜像传入相邻房间 (相邻房间不存在则停留)
fn apply_crossings(world: &mut World, crossings: &mut Vec<(String, u64)>) {
    // 需要世界房名解析
    fn room_xy(name: &str) -> Option<(i64, i64)> {
        let mut chars = name.chars();
        let h = chars.next()?;
        let rest: String = chars.collect();
        let (x_part, y_part) = rest.split_once(['N', 'S'])?;
        let x: i64 = x_part.parse().ok()?;
        let x = if h == 'W' || h == 'w' { -x - 1 } else { x };
        let y: i64 = y_part.parse().ok()?;
        let y = if rest.contains('N') || rest.contains('n') { -y - 1 } else { y };
        Some((x, y))
    }
    fn xy_name(x: i64, y: i64) -> String {
        let (hx, x) = if x < 0 { ("W", -x - 1) } else { ("E", x) };
        let (hy, y) = if y < 0 { ("N", -y - 1) } else { ("S", y) };
        format!("{hx}{x}{hy}{y}")
    }
    for (room_name, id) in crossings.drain(..) {
        let Some(room) = world.rooms.get(&room_name) else { continue };
        let Some(obj) = room.objects.get(&id) else { continue };
        let Some((rx, ry)) = room_xy(&room_name) else { continue };
        let (nx, ny, nrx, nry) = if obj.x == 0 {
            (49, obj.y, rx - 1, ry)
        } else if obj.x == ROOM_SIZE as u8 - 1 {
            (0, obj.y, rx + 1, ry)
        } else if obj.y == 0 {
            (obj.x, 49, rx, ry - 1)
        } else {
            (obj.x, 0, rx, ry + 1)
        };
        let target_name = xy_name(nrx, nry);
        if !world.rooms.contains_key(&target_name) {
            continue; // 有限世界: 无相邻房间则停留在边缘格
        }
        let mut obj = world.rooms.get_mut(&room_name).unwrap().objects.remove(&id).unwrap();
        obj.x = nx;
        obj.y = ny;
        // 官方 EVENT_EXIT: 记录在离开的房间, data.room = 进入的房间
        world
            .rooms
            .get_mut(&room_name)
            .unwrap()
            .push_event(intents::EVENT_EXIT, id, json!({ "room": target_name }));
        let target = world.rooms.get_mut(&target_name).unwrap();
        target.objects.insert(id, obj);
    }
}
