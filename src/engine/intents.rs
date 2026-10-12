//! 意图结算: JS shim 收集的意图按官方引擎语义应用。
//! 顺序: 对象 id 序逐个应用 (官方 processor.js 按 roomObjects 迭代), 房间级意图最后。

use crate::engine::constants;
use crate::engine::objects::{self, BodyPart};
use crate::engine::world::{alloc_id, Room, World};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 官方 EVENT_* 常量 (common/lib/constants.js)
pub const EVENT_ATTACK: u32 = 1;
pub const EVENT_OBJECT_DESTROYED: u32 = 2;
pub const EVENT_ATTACK_CONTROLLER: u32 = 3;
pub const EVENT_BUILD: u32 = 4;
pub const EVENT_HARVEST: u32 = 5;
pub const EVENT_HEAL: u32 = 6;
pub const EVENT_REPAIR: u32 = 7;
pub const EVENT_RESERVE_CONTROLLER: u32 = 8;
pub const EVENT_UPGRADE_CONTROLLER: u32 = 9;
pub const EVENT_EXIT: u32 = 10;
pub const EVENT_TRANSFER: u32 = 11;

/// 官方 `processor/intents/creeps/intents.js` 的 creepActions 执行顺序
const CREEP_ACTIONS: &[&str] = &[
    "drop",
    "transfer",
    "withdraw",
    "pickup",
    "heal",
    "rangedHeal",
    "dismantle",
    "attack",
    "harvest",
    "move",
    "repair",
    "build",
    "rangedMassAttack",
    "rangedAttack",
    "say",
    "suicide",
    "claimController",
    "upgradeController",
    "reserveController",
    "attackController",
    "generateSafeMode",
    "signController",
    "pull",
];

/// 官方 `checkPriorities` 互斥表: 出现表中任一意图时, 该意图不执行。
fn suppressed_by(name: &str) -> &'static [&'static str] {
    match name {
        "rangedHeal" => &["heal"],
        "attackController" => &["rangedHeal", "heal"],
        "dismantle" => &["attackController", "rangedHeal", "heal"],
        "repair" => &["dismantle", "attackController", "rangedHeal", "heal"],
        "build" => &["repair", "dismantle", "attackController", "rangedHeal", "heal"],
        "attack" => &["build", "repair", "dismantle", "attackController", "rangedHeal", "heal"],
        "harvest" => &[
            "attack",
            "build",
            "repair",
            "dismantle",
            "attackController",
            "rangedHeal",
            "heal",
        ],
        "rangedMassAttack" => &["build", "repair", "rangedHeal"],
        "rangedAttack" => &["rangedMassAttack", "build", "repair", "rangedHeal"],
        _ => &[],
    }
}

/// 意图执行序: creep 动作按官方 creepActions, 其余 (spawn/tower/link) 排在后面。
fn intent_rank(name: &str) -> usize {
    if let Some(pos) = CREEP_ACTIONS.iter().position(|n| *n == name) {
        return pos;
    }
    match name {
        // 官方 spawns/intents.js: createCreep → renewCreep → recycleCreep → setSpawnDirections → cancelSpawning
        "spawnCreep" => 100,
        "renewCreep" => 102,
        "recycleCreep" => 103,
        "setSpawnDirections" => 104,
        "cancelSpawning" => 105,
        "transferEnergy" => 110,
        _ => 200,
    }
}

pub struct IntentList {
    /// (对象 id, 意图名, 数据) — 已按对象分组前保持提交序
    pub entries: Vec<(String, String, Value)>,
}

/// shim 回传的原始意图条目 ({o: 对象id, n: 意图名, d: 数据})
#[derive(serde::Deserialize)]
pub struct IntentEntry {
    pub o: String,
    pub n: String,
    #[serde(default)]
    pub d: Option<Value>,
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

    /// 从整 tick 结果的一次性反序列化构造 (避免意图被解析两遍)
    pub fn from_entries(entries: Vec<IntentEntry>) -> IntentList {
        IntentList {
            entries: entries
                .into_iter()
                .map(|e| (e.o, e.n, e.d.unwrap_or(Value::Null)))
                .collect(),
        }
    }
}

#[allow(dead_code)]
pub struct TickEffects {
    /// 本 tick 移动意图 (roomName, objId, dx, dy, oldFatigue)
    pub moves: Vec<(String, u64, i32, i32, u32)>,
    /// 本 tick 拉拽意图 (roomName, 拉人者, 被拉者), 官方 processor/intents/creeps/pull.js
    pub pulls: Vec<(String, u64, u64)>,
    /// 单位死亡待结算 (roomName, objId)
    pub deaths: Vec<(String, u64)>,
    /// 错误日志 (结算层异常, 不应出现)
    pub errors: Vec<String>,
}

pub fn apply_intents(world: &mut World, intents: &IntentList) -> TickEffects {
    let mut effects = TickEffects {
        moves: vec![],
        pulls: vec![],
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

    // 同对象意图排序: 照搬官方 processor/intents/creeps/intents.js 的 creepActions 顺序
    let mut i = 0;
    while i < object_intents.len() {
        let id = object_intents[i].0;
        let mut j = i;
        while j < object_intents.len() && object_intents[j].0 == id {
            j += 1;
        }
        // 同名意图后者覆盖前者 (官方 intents.set)
        let mut deduped: Vec<(String, Value)> = Vec::new();
        for (_, n, d) in object_intents[i..j].iter() {
            if let Some(slot) = deduped.iter_mut().find(|(en, _)| en == n) {
                slot.1 = d.clone();
            } else {
                deduped.push((n.clone(), d.clone()));
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
    let names: Vec<&str> = intents.iter().map(|(n, _)| n.as_str()).collect();
    // 官方按 creepActions 顺序执行; 非 creep 意图排在之后
    let mut ordered: Vec<&(String, Value)> = intents.iter().collect();
    ordered.sort_by_key(|(n, _)| intent_rank(n));
    for (name, data) in ordered {
        // 官方 checkPriorities: 被互斥表压制的意图不执行
        if suppressed_by(name).iter().any(|p| names.contains(p)) {
            continue;
        }
        let room = world.rooms.get_mut(&room_name).unwrap();
        match (obj_type.as_str(), name.as_str()) {
            ("creep", "move") => {
                // 官方 move.js: direction 或 id (跟随相邻 creep, 配合 pull); id 有效时覆盖 direction
                let mut d = dir_offset(data.get("direction").and_then(Value::as_i64).unwrap_or(0) as i32);
                if let (Some(tid), Some(me)) = (id_of(data, "id"), room.objects.get(&id)) {
                    if let Some(t) = room.objects.get(&tid) {
                        let (dx, dy) = (t.x as i32 - me.x as i32, t.y as i32 - me.y as i32);
                        if t.obj_type == "creep" && dx.abs().max(dy.abs()) == 1 {
                            d = Some((dx, dy));
                        }
                    }
                }
                let Some((dx, dy)) = d else {
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
            ("creep", "pull") => {
                // 官方 pull.js: 双方都是未在孵化的 creep 且相邻, 才登记拉拽关系
                let Some(tid) = id_of(data, "id") else { continue };
                let ok = match (room.objects.get(&id), room.objects.get(&tid)) {
                    (Some(me), Some(t)) => {
                        !me.spawning
                            && t.obj_type == "creep"
                            && !t.spawning
                            && (t.x as i32 - me.x as i32).abs() <= 1
                            && (t.y as i32 - me.y as i32).abs() <= 1
                    }
                    _ => false,
                };
                if ok {
                    effects.pulls.push((room_name.clone(), id, tid));
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
                room.push_event(
                    EVENT_HARVEST,
                    id,
                    json!({ "amount": amount, "sourceId": World::id_string(target_id) }),
                );
                world.energy_harvested += amount as u64;
                // 超容量掉落
                drop_overflow(world, &room_name, id);
            }
            ("creep", "transfer") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let resource = data.get("resourceType").and_then(Value::as_str).unwrap_or("energy").to_string();
                let amount = data.get("amount").and_then(Value::as_u64).map(|a| a as u32);
                let (ox, oy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y),
                    _ => continue,
                };
                // 容量按官方 capacityForResource: creep = CARRY×50, link = 800, 结构 = 各自容量
                let (cap, target_pos) = match room.objects.get(&target_id) {
                    Some(t) if t.obj_type != "creep" || !t.spawning => {
                        (t.capacity_for_resource(&resource), (t.x, t.y))
                    }
                    _ => continue,
                };
                if cap == 0 {
                    continue;
                }
                if (target_pos.0 as i32 - ox as i32).abs() > 1 || (target_pos.1 as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                // 官方 transfer.js: 专用仓 (spawn/extension/tower/link/lab…) 按该资源计, 通用仓按总量计;
                // 持有量不足 intent.amount 时整条意图作废 (不截断)
                let target = room.objects.get(&target_id).unwrap();
                let stored = if constants::is_single_resource_store(&target.obj_type) {
                    target.store.get(&resource).copied().unwrap_or(0)
                } else {
                    objects::store_sum(&target.store)
                };
                if stored >= cap {
                    continue;
                }
                let have = room.objects.get(&id).unwrap().store.get(&resource).copied().unwrap_or(0);
                let amount = amount.unwrap_or(have);
                if have < amount {
                    continue;
                }
                let amount = amount.min(cap - stored);
                if amount == 0 {
                    continue;
                }
                let t = room.objects.get_mut(&target_id).unwrap();
                let cur = t.store.get(&resource).copied().unwrap_or(0);
                t.store.insert(resource.clone(), cur + amount);
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get(&resource).copied().unwrap_or(0) - amount;
                if left > 0 {
                    obj.store.insert(resource.clone(), left);
                } else {
                    obj.store.remove(&resource);
                }
                room.push_event(
                    EVENT_TRANSFER,
                    id,
                    json!({ "amount": amount, "resourceType": resource, "targetId": World::id_string(target_id) }),
                );
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
                let available = target.store.get("energy").copied().unwrap_or(0);
                let amount = available.min(obj_free);
                if amount == 0 {
                    continue;
                }
                // 官方 pickup.js: 扣减堆的数量, 减到 0 才删除 (部分拾取不销毁整堆)
                if amount >= available {
                    room.objects.remove(&target_id);
                } else {
                    room.objects
                        .get_mut(&target_id)
                        .unwrap()
                        .store
                        .insert("energy".into(), available - amount);
                }
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
                // shim 总会带 amount; 直接驱动引擎的原始意图缺省时视为全部
                let amount = data.get("amount").and_then(Value::as_u64).map(|a| a as u32).unwrap_or(have);
                // 官方 drop.js:13: 持有量不足时整条意图作废, 不截断
                if amount == 0 || have < amount {
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
                let (sx, sy, st, remaining) = match room.objects.get(&target_id) {
                    Some(s) if s.obj_type == "constructionSite" => (
                        s.x,
                        s.y,
                        s.structure_type.clone().unwrap_or_default(),
                        s.progress_total.saturating_sub(s.progress),
                    ),
                    _ => continue,
                };
                if (sx as i32 - ox as i32).abs() > 3 || (sy as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                // 官方 creeps/build.js:46-60: 障碍类建筑的格子上有 creep 或其他障碍时, 整次建造不执行
                if constants::OBSTACLE_OBJECT_TYPES.contains(&st.as_str()) {
                    let blocked = room.objects.values().any(|o| {
                        o.x == sx
                            && o.y == sy
                            && o.id != target_id
                            && (o.obj_type == "creep"
                                || constants::OBSTACLE_OBJECT_TYPES.contains(&o.obj_type.as_str()))
                    });
                    if blocked {
                        continue;
                    }
                }
                let effect = work.min(remaining).min(energy);
                if effect == 0 {
                    continue;
                }
                room.objects.get_mut(&target_id).unwrap().progress += effect;
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get("energy").copied().unwrap_or(0) - effect;
                obj.store.insert("energy".into(), left);
                room.push_event(
                    EVENT_BUILD,
                    id,
                    json!({ "amount": effect, "energySpent": effect, "targetId": World::id_string(target_id) }),
                );
                world.energy_built += effect as u64;
                let complete = room
                    .objects
                    .get(&target_id)
                    .map(|s| s.progress >= s.progress_total)
                    .unwrap_or(false);
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
                room.push_event(
                    EVENT_REPAIR,
                    id,
                    json!({ "amount": effect, "energySpent": spent, "targetId": World::id_string(target_id) }),
                );
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
                let now = world.tick;
                let room = world.rooms.get_mut(&room_name).unwrap();
                let (level, ctrl_pos) = match room.objects.get(&controller_id) {
                    // 官方 upgradeController.js:23: 被 attackController 封锁期间不能升级
                    Some(c) if c.obj_type == "controller" && c.mine && c.level > 0 && c.upgrade_blocked <= now => (c.level, (c.x, c.y)),
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
                        // 官方 upgradeController.js: 每次升级 safeModeAvailable +1
                        controller.safe_mode_available += 1;
                        world.rcl_timeline.entry(level + 1).or_insert(world.tick);
                    } else {
                        controller.progress = progress + effect;
                    }
                }
                controller.upgraded_this_tick = upgraded + effect;
                let obj = room.objects.get_mut(&id).unwrap();
                let left = obj.store.get("energy").copied().unwrap_or(0) - effect;
                obj.store.insert("energy".into(), left);
                room.push_event(
                    EVENT_UPGRADE_CONTROLLER,
                    id,
                    json!({ "amount": effect, "energySpent": effect, "targetId": World::id_string(controller_id) }),
                );
                world.energy_control += effect as u64;
                world.gcl_progress += effect as u64;
            }
            ("creep", "say") => {
                // 无旁观者, 不结算
            }
            ("creep", "suicide") => {
                // 官方 suicide.js → _die: 立刻从 roomObjects 删除, 本 tick 不再挡路、不再能被 transfer/withdraw
                if room.objects.get(&id).is_some_and(|o| o.spawning) {
                    continue;
                }
                if let Some(obj) = room.objects.remove(&id) {
                    world.creeps_lost += 1;
                    bury_creep(world, &room_name, obj, constants::CREEP_CORPSE_RATE);
                }
                return;
            }
            // ── 战斗 (官方 processor/intents/creeps/{attack,rangedAttack,...}.js) ──
            ("creep", "attack") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("attack", constants::ATTACK_POWER),
                    ),
                    _ => continue,
                };
                if power == 0 {
                    continue;
                }
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(t)
                        if t.id != id
                            && t.hits > 0
                            && t.obj_type != "controller"
                            && !(t.obj_type == "creep" && t.spawning) =>
                    {
                        (t.x, t.y)
                    }
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let target_id = rampart_at(room, tx, ty).unwrap_or(target_id);
                apply_damage(world, &room_name, id, target_id, power, constants::EVENT_ATTACK_TYPE_MELEE);
            }
            ("creep", "rangedAttack") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("ranged_attack", constants::RANGED_ATTACK_POWER),
                    ),
                    _ => continue,
                };
                if power == 0 {
                    continue;
                }
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(t)
                        if t.id != id
                            && t.hits > 0
                            && t.obj_type != "controller"
                            && !(t.obj_type == "creep" && t.spawning) =>
                    {
                        (t.x, t.y)
                    }
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 3 || (ty as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                let target_id = rampart_at(room, tx, ty).unwrap_or(target_id);
                apply_damage(world, &room_name, id, target_id, power, constants::EVENT_ATTACK_TYPE_RANGED);
            }
            ("creep", "rangedMassAttack") => {
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("ranged_attack", constants::RANGED_ATTACK_POWER),
                    ),
                    _ => continue,
                };
                if power == 0 {
                    continue;
                }
                // 目标: 带 user 且非我方 (enemy) 的对象; 目标格有 rampart 时被挡住 (官方 rangedMassAttack.js)
                let targets: Vec<(u64, u8, u8)> = room
                    .objects
                    .values()
                    .filter(|o| {
                        o.enemy
                            && o.hits > 0
                            && !(o.obj_type == "creep" && o.spawning)
                            && (o.x as i32 - ox as i32).abs() <= 3
                            && (o.y as i32 - oy as i32).abs() <= 3
                            && (o.obj_type == "rampart"
                                || !room
                                    .objects
                                    .values()
                                    .any(|r| r.obj_type == "rampart" && r.x == o.x && r.y == o.y))
                    })
                    .map(|o| (o.id, o.x, o.y))
                    .collect();
                for (tid, tx, ty) in targets {
                    let dist = (ox as i32 - tx as i32).abs().max((oy as i32 - ty as i32).abs());
                    let rate = constants::RANGED_MASS_ATTACK_DISTANCE_RATE
                        .iter()
                        .find(|(d, _)| *d == dist)
                        .map(|(_, r)| *r)
                        .unwrap_or(0.0);
                    let dmg = (power as f64 * rate).round() as u32;
                    if dmg == 0 {
                        continue;
                    }
                    apply_damage(world, &room_name, id, tid, dmg, constants::EVENT_ATTACK_TYPE_RANGED_MASS);
                }
            }
            ("creep", "heal") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("heal", constants::HEAL_POWER),
                    ),
                    _ => continue,
                };
                if power == 0 {
                    continue;
                }
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(t) if t.obj_type == "creep" && !t.spawning => (t.x, t.y),
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                apply_heal(world, &room_name, id, target_id, power, constants::EVENT_HEAL_TYPE_MELEE);
            }
            ("creep", "rangedHeal") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("heal", constants::RANGED_HEAL_POWER),
                    ),
                    _ => continue,
                };
                if power == 0 {
                    continue;
                }
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(t) if t.obj_type == "creep" && !t.spawning => (t.x, t.y),
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 3 || (ty as i32 - oy as i32).abs() > 3 {
                    continue;
                }
                apply_heal(world, &room_name, id, target_id, power, constants::EVENT_HEAL_TYPE_RANGED);
            }
            ("creep", "dismantle") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy, power) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (
                        o.x,
                        o.y,
                        o.calc_body_effectiveness("work", constants::DISMANTLE_POWER),
                    ),
                    _ => continue,
                };
                // 目标必须是带建造成本的结构 (官方 dismantle.js: C.CONSTRUCTION_COST[target.type])
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(t) if constants::construction_cost(&t.obj_type).is_some() => (t.x, t.y),
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                // 先重定向到 rampart (官方在计算 amount 之前重定向)
                let target_id = rampart_at(room, tx, ty).unwrap_or(target_id);
                let hits = match room.objects.get(&target_id) {
                    Some(t) => t.hits,
                    None => continue,
                };
                let amount = power.min(hits);
                if amount == 0 {
                    continue;
                }
                let gain = (amount as f64 * constants::DISMANTLE_COST).floor() as u32;
                if gain > 0 {
                    if let Some(o) = room.objects.get_mut(&id) {
                        let e = o.store.get("energy").copied().unwrap_or(0) + gain;
                        o.store.insert("energy".into(), e);
                    }
                    drop_overflow(world, &room_name, id);
                }
                apply_damage(world, &room_name, id, target_id, amount, constants::EVENT_ATTACK_TYPE_DISMANTLE);
            }
            ("creep", "attackController") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let now = world.tick;
                let (ox, oy, claim_parts) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y, o.live_parts("claim") as u64),
                    _ => continue,
                };
                let (tx, ty, has_user, has_res, upgrade_blocked) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "controller" => (
                        c.x,
                        c.y,
                        c.mine,
                        c.reservation_end_time.is_some(),
                        c.upgrade_blocked,
                    ),
                    _ => continue,
                };
                if !has_user && !has_res {
                    continue;
                }
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                if upgrade_blocked > now {
                    continue;
                }
                if has_res {
                    let effect = claim_parts * constants::CONTROLLER_RESERVE;
                    if effect == 0 {
                        continue;
                    }
                    if let Some(c) = room.objects.get_mut(&target_id) {
                        let end = c.reservation_end_time.unwrap_or(0).saturating_sub(effect);
                        c.reservation_end_time = Some(end);
                    }
                }
                if has_user {
                    let effect = claim_parts * constants::CONTROLLER_CLAIM_DOWNGRADE;
                    if effect == 0 {
                        continue;
                    }
                    if let Some(c) = room.objects.get_mut(&target_id) {
                        let dt = c.downgrade_time.unwrap_or(now).saturating_sub(effect);
                        c.downgrade_time = Some(dt);
                        c.upgrade_blocked = now + constants::CONTROLLER_ATTACK_BLOCKED_UPGRADE;
                    }
                }
                room.push_event(EVENT_ATTACK_CONTROLLER, id, json!({}));
            }
            ("creep", "reserveController") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let now = world.tick;
                let (ox, oy, claim_parts) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y, o.live_parts("claim") as u64),
                    _ => continue,
                };
                let (tx, ty, owned) = match room.objects.get(&target_id) {
                    // 官方 reserveController.js: 有主 (target.user) 的控制器不能保留
                    Some(c) if c.obj_type == "controller" => (c.x, c.y, c.mine || c.enemy),
                    _ => continue,
                };
                if owned {
                    continue;
                }
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let effect = claim_parts * constants::CONTROLLER_RESERVE;
                if effect == 0 {
                    continue;
                }
                let c = room.objects.get_mut(&target_id).unwrap();
                let end = c.reservation_end_time.unwrap_or(now + 1);
                if end + effect > now + constants::CONTROLLER_RESERVE_MAX {
                    continue;
                }
                c.reservation_end_time = Some(end + effect);
                room.push_event(EVENT_RESERVE_CONTROLLER, id, json!({ "amount": effect }));
            }
            ("creep", "claimController") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let now = world.tick;
                let (ox, oy, has_claim) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y, o.live_parts("claim") > 0),
                    _ => continue,
                };
                if !has_claim {
                    continue;
                }
                let (tx, ty, level) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "controller" => (c.x, c.y, c.level),
                    _ => continue,
                };
                if level > 0 {
                    continue;
                }
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                // GCL 检查: 官方 calcNeededGcl(已占领房间数 + 1) = MULTIPLY × 已占领数^GCL_POW
                let claimed = world
                    .rooms
                    .values()
                    .filter(|r| r.controller().map(|c| c.mine && c.level > 0).unwrap_or(false))
                    .count() as f64;
                let needed =
                    (constants::GCL_MULTIPLY as f64 * claimed.powf(constants::GCL_POW)) as u64;
                if world.gcl_progress < needed {
                    continue;
                }
                if let Some(c) = world
                    .rooms
                    .get_mut(&room_name)
                    .unwrap()
                    .objects
                    .get_mut(&target_id)
                {
                    c.mine = true;
                    c.level = 1;
                    c.progress = 0;
                    c.downgrade_time = None;
                    c.reservation_end_time = None;
                    c.upgrade_blocked = 0;
                }
                world.rcl_timeline.entry(1).or_insert(now);
            }
            ("creep", "signController") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy) = match room.objects.get(&id) {
                    Some(o) if !o.spawning => (o.x, o.y),
                    _ => continue,
                };
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "controller" => (c.x, c.y),
                    _ => continue,
                };
                if (tx as i32 - ox as i32).abs() > 1 || (ty as i32 - oy as i32).abs() > 1 {
                    continue;
                }
                let sign = data.get("sign").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(c) = room.objects.get_mut(&target_id) {
                    c.sign = if sign.is_empty() { None } else { Some(sign) };
                }
            }
            ("spawn", "spawnCreep") => {
                // 官方 spawns/create-creep.js
                let body_spec: Vec<String> = data
                    .get("body")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default();
                let name = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                // 官方: spawning 中直接返回; checkStructureAgainstController 不通过 (非我方 / 超出 RCL) 也返回
                let usable = match room.objects.get(&id) {
                    Some(spawn) => spawn.spawn_job.is_none() && spawn.mine && room.structure_active(spawn),
                    None => continue,
                };
                if !usable || name.is_empty() || body_spec.is_empty() || body_spec.len() > constants::MAX_CREEP_SIZE {
                    continue;
                }
                // 官方: directions 存在时须为数组, 去重后每个都是 1..8 的整数, 否则整个意图作废
                let directions = match data.get("directions") {
                    None | Some(Value::Null) => None,
                    Some(v) => match parse_directions(v) {
                        Some(dirs) => Some(dirs).filter(|d: &Vec<i32>| !d.is_empty()),
                        None => continue,
                    },
                };
                // 同 tick 重名: 孵化中的 creep 已落位, 直接查重即可 (官方 createdCreepNames)
                let dup = room
                    .objects
                    .values()
                    .any(|o| o.obj_type == "creep" && o.name.as_deref() == Some(name.as_str()));
                if dup {
                    continue;
                }
                let cost: u32 = body_spec.iter().map(|p| constants::bodypart_cost(p)).sum();
                // energyStructures 缺省 → 官方旧扣能路径; 给出 (哪怕为空) → 只用列出的结构
                let preferred: Option<Vec<u64>> = data.get("energyStructures").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| u64::from_str_radix(s, 16).ok())
                        .collect()
                });
                if !charge_spawn_energy(room, id, cost, preferred.as_deref()) {
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
                    directions,
                });
                world.creeps_produced += 1;
                world.energy_spawned += cost as u64;
            }
            ("spawn", "setSpawnDirections") => {
                // 官方 spawns/set-spawn-directions.js: 非空数组、去重后全为 1..8 整数才覆盖出生方向序
                let Some(directions) = data.get("directions").and_then(parse_directions) else { continue };
                if directions.is_empty() {
                    continue;
                }
                if let Some(job) = room.objects.get_mut(&id).and_then(|s| s.spawn_job.as_mut()) {
                    job.directions = Some(directions);
                }
            }
            ("spawn", "cancelSpawning") => {
                // 官方 spawns/cancel-spawning.js: 移除 spawn 格上同名的孵化中 creep, 清空 spawning (不退能量)
                let Some(spawn) = room.objects.get(&id) else { continue };
                let Some(job) = spawn.spawn_job.clone() else { continue };
                let (sx, sy) = (spawn.x, spawn.y);
                let creep_id = room
                    .objects
                    .values()
                    .find(|o| {
                        o.obj_type == "creep" && o.name.as_deref() == Some(job.name.as_str()) && o.x == sx && o.y == sy
                    })
                    .map(|o| o.id);
                if let Some(cid) = creep_id {
                    room.objects.remove(&cid);
                }
                room.objects.get_mut(&id).unwrap().spawn_job = None;
            }
            ("spawn", "renewCreep") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                // 官方 structures.js:1237-1266 / spawns/renew-creep.js
                let (sx, sy) = match room.objects.get(&id) {
                    Some(s) if s.mine && s.is_active() && s.spawn_job.is_none() => (s.x, s.y),
                    _ => continue,
                };
                let (tx, ty, body_len, has_claim, age_time, body_cost) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "creep" && !c.spawning && c.mine => (
                        c.x,
                        c.y,
                        c.body.len() as u32,
                        c.body.iter().any(|p| p.part_type == "claim"),
                        c.age_time,
                        c.body.iter().map(|p| constants::bodypart_cost(&p.part_type)).sum::<u32>(),
                    ),
                    _ => continue,
                };
                if has_claim || body_len == 0 {
                    continue;
                }
                if (tx as i32 - sx as i32).abs() > 1 || (ty as i32 - sy as i32).abs() > 1 {
                    continue;
                }
                let effect = (constants::SPAWN_RENEW_RATIO * constants::CREEP_LIFE_TIME as f64
                    / constants::CREEP_SPAWN_TIME as f64
                    / body_len as f64)
                    .floor() as u64;
                // 已满寿命: 不再恢复 (官方 ERR_FULL 分支)
                if age_time.map(|a| a + effect > world.tick + constants::CREEP_LIFE_TIME).unwrap_or(false) {
                    continue;
                }
                let cost = (constants::SPAWN_RENEW_RATIO * body_cost as f64
                    / constants::CREEP_SPAWN_TIME as f64
                    / body_len as f64)
                    .ceil() as u32;
                if !charge_spawn_energy(room, id, cost, None) {
                    continue;
                }
                if let Some(c) = room.objects.get_mut(&target_id) {
                    if let Some(a) = c.age_time.as_mut() {
                        *a += effect;
                    }
                }
                world.energy_spawned += cost as u64;
            }
            ("spawn", "recycleCreep") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                // 官方 structures.js:1269-1290 / spawns/recycle-creep.js: 回收 = 100% 掉落
                let (sx, sy) = match room.objects.get(&id) {
                    Some(s) if s.mine && s.is_active() => (s.x, s.y),
                    _ => continue,
                };
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "creep" && !c.spawning && c.mine => (c.x, c.y),
                    _ => continue,
                };
                if (tx as i32 - sx as i32).abs() > 1 || (ty as i32 - sy as i32).abs() > 1 {
                    continue;
                }
                if let Some(obj) = room.objects.remove(&target_id) {
                    world.creeps_lost += 1;
                    bury_creep(world, &room_name, obj, 1.0);
                }
            }
            ("tower", "repair") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy) = match room.objects.get(&id) {
                    Some(t)
                        if t.mine
                            && t.store.get("energy").copied().unwrap_or(0) >= constants::TOWER_ENERGY_COST =>
                    {
                        (t.x, t.y)
                    }
                    _ => continue,
                };
                let (tx, ty, repairable, full) = match room.objects.get(&target_id) {
                    Some(o) => (
                        o.x,
                        o.y,
                        constants::construction_cost(&o.obj_type).is_some(),
                        o.hits >= o.hits_max,
                    ),
                    None => continue,
                };
                if !repairable || full {
                    continue;
                }
                let amount = tower_power(constants::TOWER_POWER_REPAIR, ox, oy, tx, ty);
                if amount == 0 {
                    continue;
                }
                {
                    let t = room.objects.get_mut(&target_id).unwrap();
                    t.hits = (t.hits + amount).min(t.hits_max);
                }
                let tower = room.objects.get_mut(&id).unwrap();
                let e = tower.store.get("energy").copied().unwrap_or(0) - constants::TOWER_ENERGY_COST;
                tower.store.insert("energy".into(), e);
                room.push_event(
                    EVENT_REPAIR,
                    id,
                    json!({ "targetId": World::id_string(target_id), "amount": amount, "energySpent": constants::TOWER_ENERGY_COST }),
                );
            }
            ("tower", "heal") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy) = match room.objects.get(&id) {
                    Some(t)
                        if t.mine
                            && t.store.get("energy").copied().unwrap_or(0) >= constants::TOWER_ENERGY_COST =>
                    {
                        (t.x, t.y)
                    }
                    _ => continue,
                };
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(c) if c.obj_type == "creep" && !c.spawning => (c.x, c.y),
                    _ => continue,
                };
                let amount = tower_power(constants::TOWER_POWER_HEAL, ox, oy, tx, ty);
                if amount == 0 {
                    continue;
                }
                // 官方 towers/heal.js: 累积 _healToApply (tick 阶段统一应用)
                apply_heal(world, &room_name, id, target_id, amount, constants::EVENT_HEAL_TYPE_RANGED);
                let tower = world.rooms.get_mut(&room_name).unwrap().objects.get_mut(&id).unwrap();
                let e = tower
                    .store
                    .get("energy")
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(constants::TOWER_ENERGY_COST);
                tower.store.insert("energy".into(), e);
            }
            ("tower", "attack") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let (ox, oy) = match room.objects.get(&id) {
                    Some(t)
                        if t.mine
                            && t.store.get("energy").copied().unwrap_or(0) >= constants::TOWER_ENERGY_COST =>
                    {
                        (t.x, t.y)
                    }
                    _ => continue,
                };
                // 官方 towers/attack.js: 孵化中的 creep 不能被攻击
                let (tx, ty) = match room.objects.get(&target_id) {
                    Some(o)
                        if o.id != id
                            && o.hits > 0
                            && o.obj_type != "controller"
                            && !(o.obj_type == "creep" && o.spawning) =>
                    {
                        (o.x, o.y)
                    }
                    _ => continue,
                };
                let amount = tower_power(constants::TOWER_POWER_ATTACK, ox, oy, tx, ty);
                if amount == 0 {
                    continue;
                }
                let target_id = rampart_at(room, tx, ty).unwrap_or(target_id);
                // 官方 towers/attack.js: 走统一 _damage (creep 累积伤害, 结构即时扣血), 随后扣 10 能
                apply_damage(world, &room_name, id, target_id, amount, constants::EVENT_ATTACK_TYPE_RANGED);
                let tower = world.rooms.get_mut(&room_name).unwrap().objects.get_mut(&id).unwrap();
                let e = tower
                    .store
                    .get("energy")
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(constants::TOWER_ENERGY_COST);
                tower.store.insert("energy".into(), e);
            }
            ("controller", "unclaim") => {
                // 官方 controllers/unclaim.js (新手区不建模: 冷却总是生效)
                if let Some(c) = room.objects.get_mut(&id) {
                    if !c.mine || c.level == 0 {
                        continue;
                    }
                    c.mine = false;
                    c.level = 0;
                    c.progress = 0;
                    c.downgrade_time = None;
                    c.safe_mode = None;
                    c.safe_mode_available = 0;
                    c.safe_mode_cooldown = Some(world.tick + constants::SAFE_MODE_COOLDOWN);
                }
            }
            ("controller", "activateSafeMode") => {
                // 官方 controllers/activateSafeMode.js: 只打标记, 控制器 tick 时才真正开启
                let now = world.tick;
                if let Some(c) = room.objects.get_mut(&id) {
                    if !c.mine || c.level == 0 || c.safe_mode_available == 0 {
                        continue;
                    }
                    if c.safe_mode_cooldown.map(|t| t >= now).unwrap_or(false) || c.upgrade_blocked > now {
                        continue;
                    }
                    let threshold = (now + downgrade_for(c.level) / 2)
                        .saturating_sub(constants::CONTROLLER_DOWNGRADE_SAFEMODE_THRESHOLD);
                    if c.downgrade_time.map(|t| t < threshold).unwrap_or(false) {
                        continue;
                    }
                    c.safe_mode_activated = true;
                }
            }
            ("rampart", "setPublic") => {
                // 官方 ramparts/set-public.js
                let is_public = data.get("isPublic").map(truthy).unwrap_or(false);
                if let Some(r) = room.objects.get_mut(&id) {
                    if r.mine {
                        r.is_public = is_public;
                    }
                }
            }
            ("link", "transferEnergy") => {
                let Some(target_id) = id_of(data, "id") else { continue };
                let amount_req = data.get("amount").and_then(Value::as_u64).unwrap_or(0) as u32;
                // 官方 links/transfer.js: 同房 link 间转移, 有冷却与 3% 损耗;
                // 源 link 须通过 checkStructureAgainstController (超出 RCL 上限的 link 不能发)
                let (ox, oy, have, cooldown) = match room.objects.get(&id) {
                    Some(l) if l.mine && l.obj_type == "link" && room.structure_active(l) => {
                        (l.x, l.y, l.store.get("energy").copied().unwrap_or(0), l.cooldown)
                    }
                    _ => continue,
                };
                if target_id == id || cooldown > 0 || amount_req == 0 || have < amount_req {
                    continue;
                }
                let (tx, ty, target_energy) = match room.objects.get(&target_id) {
                    Some(t) if t.obj_type == "link" && t.mine => {
                        (t.x, t.y, t.store.get("energy").copied().unwrap_or(0))
                    }
                    _ => continue,
                };
                let cap = constants::LINK_CAPACITY;
                if target_energy >= cap {
                    continue;
                }
                // 官方处理器在目标放不下时把 amount 截到剩余容量 (API 层已对显式超量返回 ERR_FULL)
                let amount = amount_req.min(cap - target_energy).min(have);
                if amount == 0 {
                    continue;
                }
                let loss = (amount as f64 * constants::LINK_LOSS_RATIO).ceil() as u32;
                {
                    let t = room.objects.get_mut(&target_id).unwrap();
                    let te = t.store.get("energy").copied().unwrap_or(0) + amount;
                    t.store.insert("energy".into(), te.saturating_sub(loss));
                }
                let dist = (ox as i32 - tx as i32).abs().max((oy as i32 - ty as i32).abs()) as u32;
                let l = room.objects.get_mut(&id).unwrap();
                l.store.insert("energy".into(), have - amount);
                l.cooldown += constants::LINK_COOLDOWN * dist.max(1);
                room.push_event(
                    EVENT_TRANSFER,
                    id,
                    json!({ "targetId": World::id_string(target_id), "resourceType": "energy", "amount": amount }),
                );
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
            // 地形与占位检查 (官方只豁免 road/extractor; 容器不能建在墙上)
            if st != "road" && st != "extractor" && room.terrain.is_wall(x as usize, y as usize) {
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
            if st == "road" {
                // 官方 room/create-construction-site.js:42-45: 沼泽 ×5, 墙 ×150
                if room.terrain.is_wall(x as usize, y as usize) {
                    progress_total *= constants::CONSTRUCTION_COST_ROAD_WALL_RATIO;
                } else if room.terrain.is_swamp(x as usize, y as usize) {
                    progress_total *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
                }
            }
            let id = alloc_id(&mut world.next_id);
            let site = objects::construction_site(id, st, x, y, progress_total);
            room.objects.insert(id, site);
        }
        "removeConstructionSite" => {
            let Some(target_id) = id_of(data, "id") else { return };
            // 官方 room/remove-construction-site.js: 只处理工地
            if room.objects.get(&target_id).map(|o| o.obj_type != "constructionSite").unwrap_or(true) {
                return;
            }
            if let Some(removed) = room.objects.remove(&target_id) {
                let type_name = removed
                    .structure_type
                    .clone()
                    .unwrap_or_else(|| removed.obj_type.clone());
                // 官方 room/remove-construction-site.js:17-19: 拆除工地掉落 floor(progress/2) 能量
                let refund = removed.progress / 2;
                let (x, y) = (removed.x, removed.y);
                room.push_event(EVENT_OBJECT_DESTROYED, target_id, json!({ "type": type_name }));
                if refund > 0 {
                    create_dropped(world, room_name, x, y, "energy", refund);
                }
            }
        }
        "destroyStructure" => {
            // 官方 room/destroy-structure.js: 目标须有 CONSTRUCTION_COST、控制器归我方、
            // 房内没有敌方 creep / powerCreep; 摧毁走 structures/_destroy (留下 ruin), 不记事件
            let Some(target_id) = id_of(data, "id") else { return };
            let ok = match room.objects.get(&target_id) {
                Some(o) => constants::construction_cost(&o.obj_type).is_some(),
                None => false,
            };
            let owned = room.controller().map(|c| c.mine).unwrap_or(false);
            let hostiles = room
                .objects
                .values()
                .any(|o| (o.obj_type == "creep" || o.obj_type == "powerCreep") && !o.mine);
            if !ok || !owned || hostiles {
                return;
            }
            destroy_structure(world, room_name, target_id, None);
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

/// JS 真值语义 (`!!value`)
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => true,
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

/// spawn 能量扣费 (官方 spawns/_charge-energy.js)。
/// - `preferred = None`: 旧路径 —— 全部 spawn 就近优先, 再 extension 就近优先 (切比雪夫距离)
/// - `preferred = Some(ids)`: 官方 `opts.energyStructures` —— 只用列出的、属于我方且未停用的
///   spawn/extension, 按给定顺序扣, 重复 id 只保留第一次出现 (官方 `_.uniq`)
/// 被停用 (off) 的结构不参与扣费 (官方 _charge-energy.js:7-8)。
fn charge_spawn_energy(room: &mut Room, spawn_id: u64, mut cost: u32, preferred: Option<&[u64]>) -> bool {
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
        .filter(|o| o.mine && o.is_active() && (o.obj_type == "spawn" || o.obj_type == "extension"))
        .map(|o| Src {
            id: o.id,
            energy: o.store.get("energy").copied().unwrap_or(0),
            x: o.x as i32,
            y: o.y as i32,
            is_spawn: o.obj_type == "spawn",
        })
        .collect();
    let (sx, sy) = {
        let s = &room.objects[&spawn_id];
        (s.x as i32, s.y as i32)
    };
    let cheb = |s: &Src| (s.x - sx).abs().max((s.y - sy).abs());
    let ordered: Vec<Src> = match preferred {
        None => {
            let mut v = sources.clone();
            v.sort_by_key(cheb);
            // 官方顺序: 全部 spawn 就近优先, 再 extension 就近优先
            v.sort_by_key(|s| !s.is_spawn);
            v
        }
        Some(ids) => {
            let mut seen: Vec<u64> = Vec::new();
            ids.iter()
                .filter(|pid| {
                    if seen.contains(pid) {
                        false
                    } else {
                        seen.push(**pid);
                        true
                    }
                })
                .filter_map(|pid| sources.iter().find(|s| s.id == *pid).cloned())
                .collect()
        }
    };
    let ordered_total: u32 = ordered.iter().map(|s| s.energy).sum();
    if ordered.is_empty() || ordered_total < cost {
        return false;
    }
    for src in ordered {
        if cost == 0 {
            break;
        }
        let obj = room.objects.get_mut(&src.id).unwrap();
        let have = obj.store.get("energy").copied().unwrap_or(0);
        let take = have.min(cost);
        if take > 0 {
            obj.store.insert("energy".into(), have.saturating_sub(take));
            cost -= take;
        }
    }
    true
}

/// 官方 directions 校验: 数组 → 逐个转数字并去重 (保持首次出现序), 任一不是 1..8 的整数 → None
fn parse_directions(v: &Value) -> Option<Vec<i32>> {
    let arr = v.as_array()?;
    let mut out: Vec<i32> = Vec::new();
    for d in arr {
        let n = match d {
            Value::Number(n) => n.as_f64()?,
            Value::String(s) => s.trim().parse::<f64>().ok()?,
            _ => return None,
        };
        if !(1.0..=8.0).contains(&n) || n.fract() != 0.0 {
            return None;
        }
        let n = n as i32;
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Some(out)
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

/// 把 `amount` 尽量装入 (x,y) 上 hits>0 的同格容器, 返回未装下的剩余量。
/// 官方 _create-energy.js:20-31 / _die.js 的共用语义。
pub fn fill_container(room: &mut Room, x: u8, y: u8, resource: &str, amount: u32) -> u32 {
    if amount == 0 {
        return 0;
    }
    let Some(cid) = room
        .objects
        .values()
        .find(|o| o.obj_type == "container" && o.hits > 0 && o.x == x && o.y == y)
        .map(|o| o.id)
    else {
        return amount;
    };
    let obj = room.objects.get_mut(&cid).unwrap();
    // 官方 _create-energy.js:24: 容器是通用仓, 剩余空间按总量算
    let cap = obj.capacity_for_resource(resource);
    let used = objects::store_sum(&obj.store);
    let put = amount.min(cap.saturating_sub(used));
    if put > 0 {
        let cur = obj.store.get(resource).copied().unwrap_or(0);
        obj.store.insert(resource.to_string(), cur + put);
    }
    amount - put
}

/// 塔的功率随距离衰减 (官方 towers/*.js: 超出 TOWER_OPTIMAL_RANGE 后线性衰减)
fn tower_power(base: f64, ox: u8, oy: u8, tx: u8, ty: u8) -> u32 {
    let mut range = (ox as i32 - tx as i32).abs().max((oy as i32 - ty as i32).abs()) as f64;
    let mut amount = base;
    if range > constants::TOWER_OPTIMAL_RANGE {
        if range > constants::TOWER_FALLOFF_RANGE {
            range = constants::TOWER_FALLOFF_RANGE;
        }
        amount -= amount * constants::TOWER_FALLOFF * (range - constants::TOWER_OPTIMAL_RANGE)
            / (constants::TOWER_FALLOFF_RANGE - constants::TOWER_OPTIMAL_RANGE);
    }
    amount.floor().max(0.0) as u32
}

/// 目标格上的 rampart id (官方各攻击意图的 rampart 重定向规则)
fn rampart_at(room: &Room, x: u8, y: u8) -> Option<u64> {
    room.objects
        .values()
        .find(|o| o.obj_type == "rampart" && o.x == x && o.y == y)
        .map(|o| o.id)
}

/// 官方 `processor/intents/_damage.js`:
/// - creep / powerCreep: 伤害累积到 `_damageToApply`, 在 tick 阶段统一应用 (整 tick 的攻击先全部登记再结算)
/// - 结构 / 墙: 即时扣血, 归零即销毁并掉落仓储
/// - 近战攻击会触发目标反击 (hit-back): 攻击者所在格无 rampart 时, 按目标 ATTACK 部件功率反伤攻击者
fn apply_damage(
    world: &mut World,
    room_name: &str,
    attacker_id: u64,
    target_id: u64,
    amount: u32,
    attack_type: u32,
) {
    if amount == 0 {
        return;
    }
    let (target_is_creep, attack_back) = {
        let room = &world.rooms[room_name];
        let Some(target) = room.objects.get(&target_id) else { return };
        if target.hits == 0 {
            return;
        }
        let is_creep = target.obj_type == "creep";
        let mut attack_back = 0u32;
        if is_creep && attack_type == constants::EVENT_ATTACK_TYPE_MELEE {
            let shielded = room
                .objects
                .get(&attacker_id)
                .map(|a| rampart_at(room, a.x, a.y).is_some())
                .unwrap_or(false);
            if !shielded {
                attack_back = target.calc_body_effectiveness("attack", constants::ATTACK_POWER);
            }
        }
        (is_creep, attack_back)
    };

    if target_is_creep {
        let room = world.rooms.get_mut(room_name).unwrap();
        if let Some(t) = room.objects.get_mut(&target_id) {
            t.damage_to_apply += amount;
        }
    } else {
        let destroyed = {
            let room = world.rooms.get_mut(room_name).unwrap();
            let Some(t) = room.objects.get_mut(&target_id) else { return };
            t.hits = t.hits.saturating_sub(amount);
            t.hits == 0
        };
        if destroyed {
            // 官方 _damage.js: 走 structures/_destroy (仓储留在 ruin 里), 随后记 EVENT_OBJECT_DESTROYED
            if let Some(removed) = destroy_structure(world, room_name, target_id, Some(attack_type)) {
                let room = world.rooms.get_mut(room_name).unwrap();
                room.push_event(EVENT_OBJECT_DESTROYED, target_id, json!({ "type": removed.obj_type }));
            }
        }
    }

    if attack_back > 0 {
        let room = world.rooms.get_mut(room_name).unwrap();
        if let Some(a) = room.objects.get_mut(&attacker_id) {
            a.damage_to_apply += attack_back;
        }
        room.push_event(
            EVENT_ATTACK,
            target_id,
            json!({
                "targetId": World::id_string(attacker_id),
                "damage": attack_back,
                "attackType": constants::EVENT_ATTACK_TYPE_HIT_BACK,
            }),
        );
    }

    let room = world.rooms.get_mut(room_name).unwrap();
    room.push_event(
        EVENT_ATTACK,
        attacker_id,
        json!({
            "targetId": World::id_string(target_id),
            "damage": amount,
            "attackType": attack_type,
        }),
    );
}

/// 官方 `processor/intents/structures/_destroy.js`: 移除结构; 正在孵化的 spawn 连带移除孵化中的 creep;
/// 非核弹摧毁时原地留下 ruin (继承仓储, decayTime = 当前 tick + RUIN_DECAY_STRUCTURES[type] || RUIN_DECAY)。
/// 返回被移除的结构。
pub fn destroy_structure(world: &mut World, room_name: &str, id: u64, attack_type: Option<u32>) -> Option<objects::Obj> {
    let tick = world.tick;
    let ruin_id = alloc_id(&mut world.next_id);
    let room = world.rooms.get_mut(room_name)?;
    let removed = room.objects.remove(&id)?;
    if let Some(job) = &removed.spawn_job {
        let spawning = room
            .objects
            .values()
            .find(|o| o.obj_type == "creep" && o.mine == removed.mine && o.name.as_deref() == Some(job.name.as_str()))
            .map(|o| o.id);
        if let Some(cid) = spawning {
            room.objects.remove(&cid);
        }
    }
    if attack_type != Some(constants::EVENT_ATTACK_TYPE_NUKE) {
        room.objects.insert(ruin_id, objects::ruin(ruin_id, &removed, tick));
    }
    Some(removed)
}

/// 官方 heal / rangedHeal / towers.heal: 治疗量累积到 `_healToApply` (tick 阶段统一应用)
fn apply_heal(
    world: &mut World,
    room_name: &str,
    healer_id: u64,
    target_id: u64,
    amount: u32,
    heal_type: u32,
) {
    if amount == 0 {
        return;
    }
    let room = world.rooms.get_mut(room_name).unwrap();
    if let Some(t) = room.objects.get_mut(&target_id) {
        t.heal_to_apply += amount;
    }
    room.push_event(
        EVENT_HEAL,
        healer_id,
        json!({
            "targetId": World::id_string(target_id),
            "amount": amount,
            "healType": heal_type,
        }),
    );
}

/// 官方 `_drop-resources-without-space.js`: 容量缩小 (CARRY 部件受损) 后按 RESOURCES_ALL
/// 顺序掉落超容资源。
pub fn drop_resources_without_space(world: &mut World, room_name: &str, id: u64) {
    loop {
        let (over, resource, x, y) = {
            let room = &world.rooms[room_name];
            let Some(obj) = room.objects.get(&id) else { return };
            let cap = obj.store_capacity();
            let total = objects::store_sum(&obj.store);
            if total <= cap {
                return;
            }
            let Some(res) = constants::RESOURCES_ALL
                .iter()
                .find(|r| obj.store.get(**r).copied().unwrap_or(0) > 0)
            else {
                return;
            };
            (total - cap, res.to_string(), obj.x, obj.y)
        };
        let drop_amt = {
            let room = &world.rooms[room_name];
            let have = room.objects[&id].store.get(&resource).copied().unwrap_or(0);
            have.min(over)
        };
        {
            let room = world.rooms.get_mut(room_name).unwrap();
            let o = room.objects.get_mut(&id).unwrap();
            let left = o.store.get(&resource).copied().unwrap_or(0) - drop_amt;
            if left > 0 {
                o.store.insert(resource.clone(), left);
            } else {
                o.store.remove(&resource);
            }
        }
        create_dropped(world, room_name, x, y, &resource, drop_amt);
    }
}

/// 单位死亡/回收: 身体资源按剩余寿命比例回收 (官方 _die.js:39-91), 携带资源全额;
/// 先装入同格容器, 装不下的进墓碑。`drop_rate`: 自然死亡/自杀 = CREEP_CORPSE_RATE, 回收 = 1.0。
pub fn bury_creep(world: &mut World, room_name: &str, obj: objects::Obj, drop_rate: f64) {
    let tick = world.tick;
    let life_time = if obj.body.iter().any(|p| p.part_type == "claim") {
        constants::CREEP_CLAIM_LIFE_TIME
    } else {
        constants::CREEP_LIFE_TIME
    };
    let ticks_to_live = obj.age_time.map(|a| a.saturating_sub(tick)).unwrap_or(0);
    let life_rate = drop_rate * ticks_to_live as f64 / life_time as f64;

    // 身体资源 (官方: 每部件 min(CREEP_PART_MAX_ENERGY, BODYPART_COST * lifeRate), 累计后取整)
    let mut body_res: BTreeMap<String, f64> = BTreeMap::new();
    for part in &obj.body {
        let cost = constants::bodypart_cost(&part.part_type) as f64;
        let e = (cost * life_rate).min(constants::CREEP_PART_MAX_ENERGY as f64);
        *body_res.entry("energy".into()).or_insert(0.0) += e;
    }

    let mut remain: objects::Store = objects::Store::new();
    let room = world.rooms.get_mut(room_name).unwrap();
    for (resource, amount) in body_res {
        let amount = amount.floor() as u32;
        if amount > 0 {
            let left = fill_container(room, obj.x, obj.y, &resource, amount);
            if left > 0 {
                // 官方 tombstone.store[type] = (已有 || 0) + amount: 身体资源与携带资源同种时累加
                *remain.entry(resource).or_insert(0) += left;
            }
        }
    }
    for (resource, amount) in &obj.store {
        if *amount > 0 {
            let left = fill_container(room, obj.x, obj.y, resource, *amount);
            if left > 0 {
                *remain.entry(resource.clone()).or_insert(0) += left;
            }
        }
    }

    let tomb_id = alloc_id(&mut world.next_id);
    let decay = tick + obj.body.len() as u64 * constants::TOMBSTONE_DECAY_PER_PART;
    let mut tomb = objects::tombstone(
        tomb_id,
        obj.x,
        obj.y,
        obj.name.as_deref().unwrap_or(""),
        decay,
    );
    tomb.store = remain;
    room.objects.insert(tomb_id, tomb);
}

/// 资源落地: 先装入同格容器, 装不下的部分才成为地上的堆 (官方 _create-energy.js)
pub fn create_dropped(world: &mut World, room_name: &str, x: u8, y: u8, resource: &str, amount: u32) {
    if amount == 0 {
        return;
    }
    let room = world.rooms.get_mut(room_name).unwrap();
    let amount = fill_container(room, x, y, resource, amount);
    if amount == 0 {
        return;
    }
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
    // 路 / 容器 / 墙没有 owner (官方 OwnedStructure 之外), 不参与房间可见性
    let mut obj = objects::structure(id, &st, x, y, constants::is_owned_structure(&st));
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
            // 官方 creeps/build.js:261: 新建容器首次衰减计时为 +CONTAINER_DECAY_TIME (100)
            obj.next_decay_time = Some(world.tick + constants::CONTAINER_DECAY_TIME);
        }
        "tower" => obj.energy_capacity = constants::TOWER_CAPACITY,
        "terminal" => obj.energy_capacity = constants::TERMINAL_CAPACITY,
        "link" => obj.energy_capacity = constants::LINK_CAPACITY,
        "road" => {
            let mut hits = constants::ROAD_HITS;
            // 官方: 墙上路 ×150, 沼泽路 ×5
            if room.terrain.is_wall(x as usize, y as usize) {
                hits *= constants::CONSTRUCTION_COST_ROAD_WALL_RATIO;
            } else if room.terrain.is_swamp(x as usize, y as usize) {
                hits *= constants::CONSTRUCTION_COST_ROAD_SWAMP_RATIO;
            }
            obj.hits = hits;
            obj.hits_max = hits;
            obj.next_decay_time = Some(world.tick + constants::ROAD_DECAY_TIME);
        }
        "rampart" => {
            obj.hits = 1;
            obj.hits_max = constants::rampart_hits_max(room.rcl());
            obj.next_decay_time = Some(world.tick + constants::RAMPART_DECAY_TIME);
        }
        "constructedWall" => {
            obj.hits = 1;
            obj.hits_max = constants::WALL_HITS_MAX;
        }
        _ => {}
    }
    room.objects.insert(id, obj);
}
