//! 意图结算: JS shim 收集的意图按官方引擎语义应用。
//! 顺序: 对象 id 序逐个应用 (官方 processor.js 按 roomObjects 迭代), 房间级意图最后。

use crate::engine::constants;
use crate::engine::objects::{self, BodyPart};
use crate::engine::world::{alloc_id, World};
use serde_json::Value;

pub struct IntentList {
    /// (对象 id, 意图名, 数据) — 已按对象分组前保持提交序
    pub entries: Vec<(String, String, Value)>,
}

impl IntentList {
    pub fn parse(json: &str) -> IntentList {
        let mut entries = Vec::new();
        if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json) {
            for item in arr {
                let Some(obj) = item.get("o").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = item.get("n").and_then(Value::as_str) else {
                    continue;
                };
                let data = item.get("d").cloned().unwrap_or(Value::Null);
                entries.push((obj.to_string(), name.to_string(), data));
            }
        }
        IntentList { entries }
    }
}

#[allow(dead_code)]
pub struct TickEffects {
    /// 本 tick 移动意图 (roomName, objId, dx, dy, oldFatigue)
    pub moves: Vec<(String, u64, i32, i32, u32)>,
    /// 单位死亡待结算 (roomName, objId)
    pub deaths: Vec<(String, u64)>,
    /// 错误日志 (结算层异常, 不应出现)
    pub errors: Vec<String>,
}

pub fn apply_intents(world: &mut World, intents: &IntentList) -> TickEffects {
    let mut effects = TickEffects {
        moves: vec![],
        deaths: vec![],
        errors: vec![],
    };
    // 对象意图按 id 序 (等价官方 roomObjects 迭代序), 同对象多意图按提交序
    let mut object_intents: Vec<(u64, String, Value)> = Vec::new();
    let mut room_intents: Vec<(String, String, Value)> = Vec::new();
    for (obj, name, data) in &intents.entries {
        if obj == "room" {
            let room = data.get("room").and_then(Value::as_str).map(str::to_string).unwrap_or_default();
            if !room.is_empty() {
                room_intents.push((room, name.clone(), data.clone()));
            }
        } else if let Ok(id) = u64::from_str_radix(obj, 16) {
            object_intents.push((id, name.clone(), data.clone()));
        }
    }
    object_intents.sort_by(|a, b| a.0.cmp(&b.0));

    // 同对象意图排序的固定顺序 (经济动作间相对顺序影响极小, 固定即可复现)
    fn intent_rank(name: &str) -> u8 {
        match name {
            "move" => 0,
            "pickup" => 1,
            "harvest" => 2,
            "withdraw" => 3,
            "transfer" => 4,
            "build" => 5,
            "repair" => 6,
            "upgradeController" => 7,
            "drop" => 8,
            "say" => 9,
            "suicide" => 10,
            "spawnCreep" => 11,
            _ => 50,
        }
    }
    let mut i = 0;
    while i < object_intents.len() {
        let id = object_intents[i].0;
        let mut j = i;
        while j < object_intents.len() && object_intents[j].0 == id {
            j += 1;
        }
        let mut group: Vec<(String, Value)> =
            object_intents[i..j].iter().map(|(_, n, d)| (n.clone(), d.clone())).collect();
        group.sort_by_key(|(n, _)| intent_rank(n));
        // 同名意图后者覆盖前者 (官方 intents.set)
        let mut deduped: Vec<(String, Value)> = Vec::new();
        for (n, d) in group {
            if let Some(slot) = deduped.iter_mut().find(|(en, _)| *en == n) {
                slot.1 = d;
            } else {
                deduped.push((n, d));
            }
        }
        apply_object_intents(world, id, &deduped, &mut effects);
        i = j;
    }

    for (room_name, name, data) in room_intents {
        apply_room_intent(world, &room_name, &name, &data, &mut effects);
    }
    effects
}

fn apply_object_intents(world: &mut World, id: u64, intents: &[(String, Value)], effects: &mut TickEffects) {
    // 先判定对象所在房间与类型
    let Some(room_name) = world
        .rooms
        .iter()
        .find(|(_, r)| r.objects.contains_key(&id))
        .map(|(n, _)| n.clone())
    else {
        return;
    };
    let obj_type = world.rooms[&room_name].objects[&id].obj_type.clone();
    for (name, data) in intents {
        let room = world.rooms.get_mut(&room_name).unwrap();
        match (obj_type.as_str(), name.as_str()) {
            ("creep", "move") => {
                let dir = data.get("direction").and_then(Value::as_i64).unwrap_or(0) as i32;
                let Some((dx, dy)) = dir_offset(dir) else {
                    continue;
                };
                if let Some(obj) = room.objects.get(&id) {
                    if obj.spawning {
                        continue;
                    }
                    let old_fatigue = obj.fatigue;
                    let nx = obj.x as i32 + dx;
                    let ny = obj.y as i32 + dy;
                    if nx < 0 || nx > 49 || ny < 0 || ny > 49 {
                        continue;
                    }
                    // 官方 move 意图只登记目标格; 目标格上有阻挡结构直接不登记
                    let blocked = room
                        .objects
                        .values()
                        .any(|o| o.x as i32 == nx && o.y as i32 == ny && o.is_obstacle());
                    if !blocked {
                        effects.moves.push((room_name.clone(), id, dx, dy, old_fatigue));
                    }
                }
            }
            ("creep", "harvest") => {
                let target_id = id_of(data, "id");
                let Some(target_id) = target_id else { continue };
                let ((ox, oy), power) = {
                    let Some(obj) = room.objects.get(&id) else { continue };
                    if obj.spawning {
                        continue;
                    }
                    ((obj.x, obj.y), obj.work_power(constants::HARVEST_POWER))
                };
                let Some(target) = room.objects.get_mut(&target_id) else {
                    continue;
                };
                if target.obj_type != "source" {
                    continue;
                }
                if (target.x as i32 - ox as i32).abs() > 1 || (target.y as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let available = target.store.get("energy").copied().unwrap_or(0);
                if available == 0 || power == 0 {
                    continue;
                }
                let amount = available.min(power);
                room.objects.get_mut(&target_id).unwrap().store.insert("energy".into(), available - amount);
                let obj = room.objects.get_mut(&id).unwrap();
                let carried = obj.store.get("energy").copied().unwrap_or(0) + amount;
                obj.store.insert("energy".into(), carried);
                world.energy_harvested += amount as u64;
                // 超容量掉落
                drop_overflow(world, &room_name, id);
            }
            ("creep", "transfer") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let resource = data.get("resourceType").and_then(Value::as_str).unwrap_or("energy").to_string();
                let amount = data.get("amount").and_then(Value::as_u64).unwrap_or(0) as u32;
                let (ox, oy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y),
                    _ => continue,
                };
                let (target_capacity, target_pos, target_store_capacity) = match room.objects.get(&target_id) {
                    Some(t) if t.obj_type != "creep" || !t.spawning => {
                        (t.energy_capacity, (t.x, t.y), t.store_capacity())
                    }
                    _ => continue,
                };
                if (target_pos.0 as i32 - ox as i32).abs() > 1 || (target_pos.1 as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let cap = if resource == "energy" { target_capacity } else { target_store_capacity };
                if cap == 0 {
                    continue;
                }
                let stored = room.objects.get(&target_id).unwrap().store.get(&resource).copied().unwrap_or(0);
                if stored >= cap {
                    continue;
                }
                let have = room.objects.get(&id).unwrap().store.get(&resource).copied().unwrap_or(0);
                let mut amount = amount.min(have);
                amount = amount.min(cap - stored);
                if amount == 0 {
                    continue;
                }
                room.objects.get_mut(&target_id).unwrap().store.insert(resource.clone(), stored + amount);
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get(&resource).copied().unwrap_or(0) - amount;
                if left > 0 {
                    obj.store.insert(resource, left);
                } else {
                    obj.store.remove(&resource);
                }
            }
            ("creep", "withdraw") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let resource = data.get("resourceType").and_then(Value::as_str).unwrap_or("energy").to_string();
                let req = data.get("amount").and_then(Value::as_u64).unwrap_or(u32::MAX as u64) as u32;
                let ((ox, oy), obj_free) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => ((o.x, o.y), o.store_capacity().saturating_sub(objects::store_sum(&o.store))),
                    _ => continue,
                };
                let Some(target) = room.objects.get(&target_id) else { continue };
                if (target.x as i32 - ox as i32).abs() > 1 || (target.y as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let available = target.store.get(&resource).copied().unwrap_or(0);
                let amount = req.min(obj_free).min(available);
                if amount == 0 {
                    continue;
                }
                room.objects.get_mut(&target_id).unwrap().store.insert(resource.clone(), available - amount);
                let obj = room.objects.get_mut(&id).unwrap();
                let carried = obj.store.get(&resource).copied().unwrap_or(0) + amount;
                obj.store.insert(resource, carried);
            }
            ("creep", "pickup") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let ((ox, oy), obj_free) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => ((o.x, o.y), o.store_capacity().saturating_sub(objects::store_sum(&o.store))),
                    _ => continue,
                };
                let Some(target) = room.objects.get(&target_id) else { continue };
                if target.obj_type != "energy" {
                    continue;
                }
                if (target.x as i32 - ox as i32).abs() > 1 || (target.y as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let amount = target.store.get("energy").copied().unwrap_or(0).min(obj_free);
                if amount == 0 {
                    continue;
                }
                room.objects.remove(&target_id);
                let obj = room.objects.get_mut(&id).unwrap();
                let carried = obj.store.get("energy").copied().unwrap_or(0) + amount;
                obj.store.insert("energy".into(), carried);
            }
            ("creep", "drop") => {
                let resource = data.get("resourceType").and_then(Value::as_str).unwrap_or("energy").to_string();
                let have = match room.objects.get(&id) {
                    Some(o) if !o.spawning => o.store.get(&resource).copied().unwrap_or(0),
                    _ => continue,
                };
                let amount = data.get("amount").and_then(Value::as_u64).unwrap_or(0) as u32;
                let amount = if amount == 0 { have } else { amount.min(have) };
                if amount == 0 {
                    continue;
                }
                let (x, y) = {
                    let obj = room.objects.get(&id).unwrap();
                    (obj.x, obj.y)
                };
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get(&resource).copied().unwrap_or(0) - amount;
                if left > 0 {
                    obj.store.insert(resource.clone(), left);
                } else {
                    obj.store.remove(&resource);
                }
                create_dropped(world, &room_name, x, y, &resource, amount);
            }
            ("creep", "build") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let ((ox, oy), work, energy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning && o.store.get("energy").copied().unwrap_or(0) > 0 => {
                        ((o.x, o.y), o.work_power(constants::BUILD_POWER), o.store.get("energy").copied().unwrap_or(0))
                    }
                    _ => continue,
                };
                let Some(site) = room.objects.get_mut(&target_id) else { continue };
                if site.obj_type != "constructionSite" {
                    continue;
                }
                if (site.x as i32 - ox as i32).abs() > 3 || (site.y as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                let remaining = site.progress_total - site.progress;
                let effect = work.min(remaining).min(energy);
                if effect == 0 {
                    continue;
                }
                site.progress += effect;
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get("energy").copied().unwrap_or(0) - effect;
                obj.store.insert("energy".into(), left);
                world.energy_built += effect as u64;
                let complete = room.objects.get(&target_id).unwrap().progress
                    >= room.objects.get(&target_id).unwrap().progress_total;
                if complete {
                    complete_site(world, &room_name, target_id);
                }
            }
            ("creep", "repair") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let ((ox, oy), power, energy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning && o.store.get("energy").copied().unwrap_or(0) > 0 => {
                        ((o.x, o.y), o.work_power(constants::REPAIR_POWER), o.store.get("energy").copied().unwrap_or(0))
                    }
                    _ => continue,
                };
                let Some(target) = room.objects.get_mut(&target_id) else { continue };
                if target.obj_type == "controller" || target.obj_type == "constructionSite" {
                    continue;
                }
                if (target.x as i32 - ox as i32).abs() > 3 || (target.y as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                // 1 能量修 100 血 (REPAIR_COST 0.01)
                let effect = power.min(target.hits_max - target.hits).min(energy * 100);
                if effect == 0 {
                    continue;
                }
                target.hits += effect;
                let spent = (effect as f64 * constants::REPAIR_COST).ceil() as u32;
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get("energy").copied().unwrap_or(0).saturating_sub(spent);
                obj.store.insert("energy".into(), left);
            }
            ("creep", "upgradeController") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let ((ox, oy), power, energy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning && o.store.get("energy").copied().unwrap_or(0) > 0 => {
                        ((o.x, o.y), o.work_power(constants::UPGRADE_CONTROLLER_POWER), o.store.get("energy").copied().unwrap_or(0))
                    }
                    _ => continue,
                };
                let controller_id = target_id;
                let (level, ctrl_pos) = match room.objects.get(&controller_id) {
                    Some(c) if c.obj_type == "controller" && c.mine && c.level > 0 => (c.level, (c.x, c.y)),
                    _ => continue,
                };
                if (ctrl_pos.0 as i32 - ox as i32).abs() > 3 || (ctrl_pos.1 as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                let (progress, upgraded, downgrade_time) = {
                    let c = room.objects.get(&controller_id).unwrap();
                    (c.progress, c.upgraded_this_tick, c.downgrade_time)
                };
                let mut effect = power.min(energy);
                if level == 8 {
                    let limit = constants::CONTROLLER_MAX_UPGRADE_PER_TICK;
                    if upgraded >= limit {
                        continue;
                    }
                    effect = effect.min(limit - upgraded);
                }
                if effect == 0 {
                    continue;
                }
                // 升级时间窗检查: 近期未升级的控制器不能跳级进度 (官方规则)
                let next_req = controller_levels(level);
                let down = downgrade_for(level);
                let can_level = downgrade_time
                    .map(|dt| dt + constants::CONTROLLER_DOWNGRADE_RESTORE >= world.tick + down)
                    .unwrap_or(true);
                let controller = room.objects.get_mut(&controller_id).unwrap();
                if level < 8 {
                    if progress + effect >= next_req && can_level {
                        controller.progress = progress + effect - next_req;
                        controller.level = level + 1;
                        controller.downgrade_time =
                            Some(world.tick + downgrade_for(level + 1) / 2);
                        if controller.level == 8 {
                            controller.progress = 0;
                        }
                        world.rcl_timeline.entry(level + 1).or_insert(world.tick);
                    } else {
                        controller.progress = progress + effect;
                    }
                }
                controller.upgraded_this_tick = upgraded + effect;
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get("energy").copied().unwrap_or(0) - effect;
                obj.store.insert("energy".into(), left);
                world.energy_control += effect as u64;
                world.gcl_progress += effect as u64;
            }
            ("creep", "say") => {
                // 无旁观者, 不结算
            }
            ("creep", "suicide") => {
                effects.deaths.push((room_name.clone(), id));
            }
            ("spawn", "spawnCreep") => {
                let body_spec: Vec<String> = data
                    .get("body")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default();
                let name = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let spawn_busy = match room.objects.get(&id) {
                    Some(spawn) => spawn.spawn_job.is_some(),
                    None => continue,
                };
                if spawn_busy || name.is_empty() || body_spec.is_empty() || body_spec.len() > constants::MAX_CREEP_SIZE {
                    continue;
                }
                let cost: u32 = body_spec.iter().map(|p| constants::bodypart_cost(p)).sum();
                if !charge_spawn_energy(room, id, cost) {
                    continue;
                }
                let need_time = constants::CREEP_SPAWN_TIME * body_spec.len() as u64;
                // 孵化中的 creep 对象立即落位 (engine: create-creep 插入 spawning:true)
                let creep_id = alloc_id(&mut world.next_id);
                let body: Vec<BodyPart> = body_spec
                    .iter()
                    .map(|p| BodyPart { part_type: p.clone(), hits: 100 })
                    .collect();
                let (sx, sy) = (room.objects[&id].x, room.objects[&id].y);
                let mut creep = objects::creep_obj(creep_id, &name, sx, sy, body);
                creep.spawning = true;
                creep.mine = true;
                room.objects.insert(creep_id, creep);
                let spawn = room.objects.get_mut(&id).unwrap();
                spawn.spawn_job = Some(objects::SpawnJob {
                    name,
                    need_time,
                    spawn_time: world.tick + need_time,
                });
                world.creeps_produced += 1;
                world.energy_spawned += cost as u64;
            }
            _ => {}
        }
    }
}

fn apply_room_intent(world: &mut World, room_name: &str, name: &str, data: &Value, _effects: &mut TickEffects) {
    let Some(room) = world.rooms.get_mut(room_name) else { return };
    match name {
        "createConstructionSite" => {
            let x = data.get("x").and_then(Value::as_u64).unwrap_or(99) as u8;
            let y = data.get("y").and_then(Value::as_u64).unwrap_or(99) as u8;
            let st = data.get("structureType").and_then(Value::as_str).unwrap_or("");
            if x == 0 || x >= 49 || y == 0 || y >= 49 {
                return;
            }
            let Some(base_cost) = constants::construction_cost(st) else { return };
            // RCL 数量上限 (建成的 + 工地)
            let rcl = room.rcl();
            let limit = constants::controller_structure_limit(st, rcl) as usize;
            if st != "road" && room.count_of_type(st) >= limit {
                return;
            }
            // 地形与占位检查
            if st != "road" && st != "extractor" && st != "container" && room.terrain.is_wall(x as usize, y as usize) {
                return;
            }
            if st != "container" && room.site_blocked(st, x, y) {
                return;
            }
            // 边界规则: 非路/容器的结构在 1/48 行列时, 外圈对应 3 格需为墙
            if st != "road" && st != "container" && (x == 1 || x == 48 || y == 1 || y == 48) {
                let tiles: Vec<(i32, i32)> = if x == 1 {
                    vec![(0, y as i32 - 1), (0, y as i32), (0, y as i32 + 1)]
                } else if x == 48 {
                    vec![(49, y as i32 - 1), (49, y as i32), (49, y as i32 + 1)]
                } else if y == 1 {
                    vec![(x as i32 - 1, 0), (x as i32, 0), (x as i32 + 1, 0)]
                } else {
                    vec![(x as i32 - 1, 49), (x as i32, 49), (x as i32 + 1, 49)]
                };
                if tiles.iter().any(|&(tx, ty)| {
                    !(0..50).contains(&tx)
                        || !(0..50).contains(&ty)
                        || !room.terrain.is_wall(tx as usize, ty as usize)
                }) {
                    return;
                }
            }
            let mut progress_total = base_cost;
            if st == "road" && room.terrain.is_swamp(x as usize, y as usize) {
                progress_total *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
            }
            let id = alloc_id(&mut world.next_id);
            let site = objects::construction_site(id, st, x, y, progress_total);
            room.objects.insert(id, site);
        }
        "removeConstructionSite" | "destroyStructure" => {
            let Some(target_id) = id_of(data, "id") else { return };
            room.objects.remove(&target_id);
        }
        "createFlag" => {
            // 旗标仅做最小支持: 不入 world (bot 未使用)
        }
        _ => {}
    }
}

// ── 辅助 ──

fn dir_offset(dir: i32) -> Option<(i32, i32)> {
    match dir {
        1 => Some((0, -1)),
        2 => Some((1, -1)),
        3 => Some((1, 0)),
        4 => Some((1, 1)),
        5 => Some((0, 1)),
        6 => Some((-1, 1)),
        7 => Some((-1, 0)),
        8 => Some((-1, -1)),
        _ => None,
    }
}

fn id_of(data: &Value, key: &str) -> Option<u64> {
    data.get(key)?.as_str().and_then(|s| u64::from_str_radix(s, 16).ok())
}

fn controller_levels(level: u8) -> u32 {
    constants::CONTROLLER_LEVELS
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, p)| *p)
        .unwrap_or(u32::MAX)
}

fn downgrade_for(level: u8) -> u64 {
    constants::CONTROLLER_DOWNGRADE
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, t)| *t)
        .unwrap_or(20_000)
}

/// spawn 能量扣费: 先就近 spawn, 再就近 extension (官方 _charge-energy 旧路径, 切比雪夫距离)
fn charge_spawn_energy(room: &mut crate::engine::world::Room, spawn_id: u64, mut cost: u32) -> bool {
    #[derive(Clone)]
    struct Src {
        id: u64,
        energy: u32,
        x: i32,
        y: i32,
        is_spawn: bool,
    }
    let sources: Vec<Src> = room
        .objects
        .values()
        .filter(|o| o.mine && (o.obj_type == "spawn" || o.obj_type == "extension"))
        .map(|o| Src {
            id: o.id,
            energy: o.store.get("energy").copied().unwrap_or(0),
            x: o.x as i32,
            y: o.y as i32,
            is_spawn: o.obj_type == "spawn",
        })
        .collect();
    let total: u32 = sources.iter().map(|s| s.energy).sum();
    if total < cost {
        return false;
    }
    let (sx, sy) = {
        let s = &room.objects[&spawn_id];
        (s.x as i32, s.y as i32)
    };
    let cheb = |s: &Src| (s.x - sx).abs().max((s.y - sy).abs());
    let mut ordered = sources.clone();
    ordered.sort_by_key(cheb);
    // 官方顺序: 全部 spawn 就近优先, 再 extension 就近优先
    ordered.sort_by_key(|s| !s.is_spawn);
    for src in ordered {
        if cost == 0 {
            break;
        }
        let take = src.energy.min(cost);
        if take > 0 {
            let obj = room.objects.get_mut(&src.id).unwrap();
            let left = obj.store.get("energy").copied().unwrap_or(0) - take;
            obj.store.insert("energy".into(), left);
            cost -= take;
        }
    }
    true
}

/// 采集超容量 → 原地掉落 (官方 harvest 溢出规则)
fn drop_overflow(world: &mut World, room_name: &str, id: u64) {
    let Some(room) = world.rooms.get_mut(room_name) else { return };
    let (over, x, y) = {
        let obj = &room.objects[&id];
        let over = objects::store_sum(&obj.store).saturating_sub(obj.store_capacity());
        (over, obj.x, obj.y)
    };
    if over > 0 {
        let obj = room.objects.get_mut(&id).unwrap();
        let left = obj.store.get("energy").copied().unwrap_or(0) - over;
        obj.store.insert("energy".into(), left);
        create_dropped(world, room_name, x, y, "energy", over);
    }
}

pub fn create_dropped(world: &mut World, room_name: &str, x: u8, y: u8, resource: &str, amount: u32) {
    if amount == 0 {
        return;
    }
    let room = world.rooms.get_mut(room_name).unwrap();
    let existing = room
        .objects
        .values()
        .find(|o| o.obj_type == "energy" && o.x == x && o.y == y)
        .map(|o| o.id);
    if let Some(eid) = existing {
        let obj = room.objects.get_mut(&eid).unwrap();
        let amt = obj.store.get(resource).copied().unwrap_or(0) + amount;
        obj.store.insert(resource.to_string(), amt);
    } else {
        let id = alloc_id(&mut world.next_id);
        let mut pile = objects::dropped_energy(id, x, y, amount);
        if resource != "energy" {
            pile.store.remove("energy");
            pile.store.insert(resource.to_string(), amount);
        }
        room.objects.insert(id, pile);
    }
}

/// 工地完成 → 建成结构 (engine build.js 的结构初始化表)
fn complete_site(world: &mut World, room_name: &str, site_id: u64) {
    let (st, x, y) = {
        let room = world.rooms.get_mut(room_name).unwrap();
        let site = &room.objects[&site_id];
        (site.structure_type.clone().unwrap_or_default(), site.x, site.y)
    };
    let room = world.rooms.get_mut(room_name).unwrap();
    room.objects.remove(&site_id);
    let id = alloc_id(&mut world.next_id);
    let mut obj = objects::structure(id, &st, x, y, true);
    match st.as_str() {
        "spawn" => {
            obj.energy_capacity = constants::SPAWN_ENERGY_CAPACITY;
        }
        "extension" => {
            obj.energy_capacity = constants::extension_capacity(room.rcl());
        }
        "storage" => obj.energy_capacity = constants::STORAGE_CAPACITY,
        "container" => {
            obj.energy_capacity = constants::CONTAINER_CAPACITY;
            obj.next_decay_time = Some(world.tick + constants::CONTAINER_DECAY_TIME_OWNED);
        }
        "tower" => obj.energy_capacity = constants::TOWER_CAPACITY,
        "terminal" => obj.energy_capacity = constants::TERMINAL_CAPACITY,
        "road" => {
            let mut hits = constants::ROAD_HITS;
            if room.terrain.is_swamp(x as usize, y as usize) {
                hits *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
            }
            obj.hits = hits;
            obj.hits_max = hits;
            obj.next_decay_time = Some(world.tick + constants::ROAD_DECAY_TIME);
        }
        "rampart" => {
            obj.hits = 1;
            obj.hits_max = 300_000;
            obj.next_decay_time = Some(world.tick + 100);
        }
        "constructedWall" => {
            obj.hits = 1;
            obj.hits_max = constants::WALL_HITS_MAX;
        }
        _ => {}
    }
    room.objects.insert(id, obj);
}
