//! Tick 推进: 对齐官方 processor.js 的阶段顺序。
//! 意图结算 → 移动仲裁 → 逐对象 tick (掉落衰减/Source 再生/creep 位移与寿命/
//! spawn 出生与自回能/控制器降级/道路容器衰减/墓碑消散) → 时间前进。

use crate::constants;
use crate::intents::{self, IntentList, TickEffects};
use crate::movement::{self, Movement};
use crate::objects;
use crate::terrain::ROOM_SIZE;
use crate::world::World;
use std::collections::BTreeMap;

pub struct TickOutcome {
    pub creep_deaths: Vec<String>,
}

pub fn run_tick(world: &mut World, movement: &mut Movement, intents: &IntentList) -> TickOutcome {
    let tick = world.tick;

    // 每 tick 重置
    for room in world.rooms.values_mut() {
        room.refresh_extension_capacity();
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
                movement.add(*id, nx, ny, *old_fatigue);
            }
        }
    }

    // 2) 每房间移动仲裁
    for room in world.rooms.values() {
        movement.check(room);
    }

    // 3) 逐对象 tick (id 序)
    let mut deaths: Vec<String> = Vec::new();
    apply_suicides(world, &effects);
    let mut pending_crossings: Vec<(String, u64)> = Vec::new();

    let room_names: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names {
        // 先按类型分层处理 (engine 逐对象 tick, 对象类型互不交叠, 分层等价且借用友好)
        tick_dropped_energy(world, room_name);
        tick_sources(world, room_name);
        tick_creeps(world, room_name, &mut movement, &mut deaths, &mut pending_crossings);
        tick_spawns(world, room_name, &mut movement);
        tick_controllers(world, room_name);
        tick_decay_structures(world, room_name);
        tick_tombstones(world, room_name);
    }
    apply_crossings(world, &mut pending_crossings);

    let _ = tick;
    world.tick += 1;
    TickOutcome { creep_deaths: deaths }
}

fn apply_suicides(world: &mut World, effects: &TickEffects) {
    for (room_name, id) in &effects.deaths {
        if let Some(room) = world.rooms.get_mut(room_name) {
            if let Some(obj) = room.objects.remove(id) {
                world.creeps_lost += 1;
                let tomb_id = world.alloc_id();
                let decay = world.tick + obj.body.len() as u64 * constants::TOMBSTONE_DECAY_PER_PART;
                let mut tomb = objects::tombstone(tomb_id, obj.x, obj.y, obj.name.as_deref().unwrap_or(""), decay);
                tomb.store = obj.store.clone();
                room.objects.insert(tomb_id, tomb);
            }
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
    let owned = world.rooms[room_name]
        .controller()
        .map(|c| c.mine)
        .unwrap_or(false);
    let tick = world.tick;
    let room = world.rooms.get_mut(room_name).unwrap();
    for obj in room.objects.values_mut() {
        if obj.obj_type != "source" {
            continue;
        }
        // 自有房间 3000 容量, 中立 1500
        let capacity = if owned {
            constants::SOURCE_ENERGY_CAPACITY
        } else {
            constants::SOURCE_ENERGY_NEUTRAL_CAPACITY
        };
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
) {
    let ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "creep" && !o.spawning)
        .map(|o| o.id)
        .collect();
    let tick = world.tick;
    for id in ids {
        // 位移
        movement.execute(world.rooms.get_mut(room_name).unwrap(), id);
        let room = world.rooms.get_mut(room_name).unwrap();
        let Some(obj) = room.objects.get_mut(&id) else { continue };

        // 边缘格: 立即准备越房 (官方 interRoom; 无法停留于出口格)
        let at_edge = obj.x == 0 || obj.x == 49 || obj.y == 0 || obj.y == 49;
        if at_edge {
            crossings.push((room_name.clone(), id));
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
            let (x, y) = (obj.x, obj.y);
            let store = obj.store.clone();
            let body_len = obj.body.len();
            let age_time = obj.age_time.unwrap_or(tick);
            let life = if body_len > 0 && obj.body.iter().any(|p| p.part_type == "claim") {
                constants::CREEP_CLAIM_LIFE_TIME
            } else {
                constants::CREEP_LIFE_TIME
            };
            let room = world.rooms.get_mut(room_name).unwrap();
            room.objects.remove(&id);
            world.creeps_lost += 1;
            deaths.push(name.clone());
            // 墓碑: 携带资源 + 尸体部件返还 (corpse_rate × 剩余寿命比, 自然死亡≈0)
            let tomb_id = world.alloc_id();
            let decay = tick + body_len as u64 * constants::TOMBSTONE_DECAY_PER_PART;
            let mut tomb = objects::tombstone(tomb_id, x, y, &name, decay);
            tomb.store = store;
            let ttl_left = age_time.saturating_sub(tick + 1) as f64;
            let life_rate = constants::CREEP_CORPSE_RATE * ttl_left / life as f64;
            let _ = life_rate; // 部件返还: 经济场景中自然死亡返还≈0, 暂不计入
            room.objects.insert(tomb_id, tomb);
            continue;
        }

        // 疲劳恢复
        movement::recover_fatigue(obj);
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
                let (x, y) = {
                    let room = &world.rooms[room_name];
                    let spawn = &room.objects[&spawn_id];
                    movement::find_birth_tile(room, spawn, movement)
                };
                let room = world.rooms.get_mut(room_name).unwrap();
                if let Some((x, y)) = x {
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
        // 自动能量回复
        let room = world.rooms.get_mut(room_name).unwrap();
        let energy_available = room.energy_available();
        let spawn = room.objects.get_mut(&spawn_id).unwrap();
        if spawn.mine
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
            // 降级后保留 90% 升到下一级的进度 (官方规则)
            let keep = ((controller_levels(new_level) as f64) * 0.9).round() as u32;
            controller.progress = controller.progress.min(keep).max(keep);
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

/// 道路/容器衰减
fn tick_decay_structures(world: &mut World, room_name: &str) {
    let tick = world.tick;
    let owned = world.rooms[room_name]
        .controller()
        .map(|c| c.mine && c.level > 0)
        .unwrap_or(false);
    let ids: Vec<u64> = world.rooms[room_name]
        .objects
        .values()
        .filter(|o| o.obj_type == "road" || o.obj_type == "container")
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
        let decay_amount = if obj.obj_type == "road" {
            let mut amount = constants::ROAD_DECAY_AMOUNT as u32;
            if room.terrain.is_swamp(obj.x as usize, obj.y as usize) {
                amount *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
            }
            amount
        } else {
            constants::CONTAINER_DECAY
        };
        obj.hits = obj.hits.saturating_sub(decay_amount);
        if obj.hits == 0 {
            let store = obj.store.clone();
            let (x, y) = (obj.x, obj.y);
            let obj_type = obj.obj_type.clone();
            room.objects.remove(&id);
            for (resource, amount) in store {
                if amount > 0 {
                    intents::create_dropped(world, room_name, x, y, &resource, amount);
                }
            }
            let _ = obj_type;
        } else {
            let interval = if obj.obj_type == "road" {
                constants::ROAD_DECAY_TIME
            } else if owned {
                constants::CONTAINER_DECAY_TIME_OWNED
            } else {
                constants::CONTAINER_DECAY_TIME
            };
            obj.next_decay_time = Some(tick + interval);
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
        let x: i64 = x_part[1..].parse().ok()?;
        let x = if h == 'W' || h == 'w' { -x - 1 } else { x };
        let y: i64 = y_part[1..].parse().ok()?;
        let y = if rest.contains('N') { -y - 1 } else { y };
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
        let obj = world.rooms.get_mut(&room_name).unwrap().objects.remove(&id).unwrap();
        let target = world.rooms.get_mut(&target_name).unwrap();
        obj.x = nx;
        obj.y = ny;
        target.objects.insert(id, obj);
    }
}
