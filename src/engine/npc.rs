//! NPC: 官方服务端管理的敌对对象 —— invader 袭击 / source keeper / power bank。
//!
//! 语义来源: docs.screeps.com/power.html (Power Bank 性质) + wiki.screepspl.us/Source_Keeper
//! (keeper 生成/守卫行为) + 社区观测的 invader 行为 (未占领房间定期袭击)。
//!
//! - **invader**: 按场景 `npcs.invaders` 的周期从房间边缘生成小队, 主动搜索并攻击
//!   最近的玩家 creep / 结构, 到达 `lifetime` 后撤离消失 (世界官方约 1.5 万 tick)。
//! - **source keeper**: keeperLair 每 `KEEPER_SPAWN_INTERVAL` tick (受扰后重新计时)
//!   生成 2 个 keeper; keeper 守卫最近的 source, 不追击, 只打进入 `KEEPER_AGGRO_RANGE`
//!   格的玩家 creep; 新 keeper 生成时旧的自杀 (wiki: the old one will suicide)。
//! - **powerBank**: 可摧毁结构, 内含 power; 对它的伤害 `POWER_BANK_HIT_BACK` 比例
//!   反弹给攻击者; hits/power 在 `POWER_BANK_DECAY` tick 内线性衰减;
//!   摧毁掉落全部 power, `POWER_BANK_RESPAWN_TIME` 后原地重生。

use crate::engine::constants;
use crate::engine::intents::IntentList;
use crate::engine::objects::{creep_obj, BodyPart, Obj};
use crate::engine::world::{alloc_id, World};
use serde_json::{json, Value};

/// 场景 `npcs.invaders` 配置 (缺省 = 无 invader 袭击)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InvaderConfig {
    /// 侵袭周期 (tick)
    #[serde(default = "default_every")]
    pub every: u64,
    /// 首次侵袭 tick
    #[serde(default = "default_first")]
    pub first: u64,
    /// invader 存活 tick 数 (到达后撤离消失)
    #[serde(default = "default_lifetime")]
    pub lifetime: u64,
    /// 每波 invader 数量
    #[serde(default = "default_squad")]
    pub squad: usize,
}

fn default_every() -> u64 {
    constants::INVADER_DEFAULT_EVERY
}
fn default_first() -> u64 {
    constants::INVADER_DEFAULT_FIRST
}
fn default_lifetime() -> u64 {
    constants::INVADER_DEFAULT_LIFETIME
}
fn default_squad() -> usize {
    constants::INVADER_DEFAULT_SQUAD
}

/// 场景 `npcs` 段: invader 侵袭开关 (keeper/powerBank 由场景结构直接放置驱动)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct NpcConfig {
    #[serde(default)]
    pub invaders: Option<InvaderConfig>,
}

/// 世界构建期: keeperLair 初始化 (下次生成 tick / 守卫目标)。
/// 新世界首个生成排在 +KEEPER_SPAWN_INTERVAL, 给场景一个起步缓冲。
pub fn prime_lairs(world: &mut World) {
    let room_names: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names {
        let Some(room) = world.rooms.get_mut(room_name) else {
            continue;
        };
        let sources: Vec<(u64, u8, u8)> = room
            .objects
            .values()
            .filter(|o| o.obj_type == "source")
            .map(|o| (o.id, o.x, o.y))
            .collect();
        if sources.is_empty() {
            continue;
        }
        let lairs: Vec<u64> = room
            .objects
            .values()
            .filter(|o| o.obj_type == "keeperLair")
            .map(|o| o.id)
            .collect();
        for lair_id in lairs {
            let (lx, ly) = {
                let l = &room.objects[&lair_id];
                (l.x, l.y)
            };
            // 守卫最近的 source (官方 keepers "move in range of the Source they are protecting")
            let guard = sources
                .iter()
                .min_by_key(|(_, sx, sy)| {
                    (*sx as i32 - lx as i32).abs().max((*sy as i32 - ly as i32).abs())
                })
                .map(|(id, _, _)| *id);
            let lair = room.objects.get_mut(&lair_id).unwrap();
            lair.guard_source = guard;
            lair.next_spawn_time = Some(world.tick + constants::KEEPER_SPAWN_INTERVAL);
        }
    }
}

/// 世界构建期: powerBank 初始化 (线性衰减排程)。
pub fn prime_power_banks(world: &mut World) {
    let room_names: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names {
        let Some(room) = world.rooms.get_mut(room_name) else {
            continue;
        };
        let bank_ids: Vec<u64> = room
            .objects
            .values()
            .filter(|o| o.obj_type == "powerBank")
            .map(|o| o.id)
            .collect();
        for id in bank_ids {
            let bank = room.objects.get_mut(&id).unwrap();
            if bank.hits_max == 0 {
                bank.hits_max = constants::POWER_BANK_HITS;
                bank.hits = constants::POWER_BANK_HITS;
            }
            bank.next_decay_time = Some(world.tick + constants::POWER_BANK_DECAY_STEP);
        }
    }
}

/// 每 tick: NPC 的意图 (在 bot 意图之后、结算之前合并进同一次 apply_intents)。
/// keeper / invader 的移动与攻击因此自动走官方意图结算管线 (疲劳/碰撞仲裁/伤害累积)。
pub fn npc_intents(world: &World) -> IntentList {
    let mut entries: Vec<(String, String, Value)> = Vec::new();
    for (room_name, room) in &world.rooms {
        for obj in room.objects.values() {
            let Some(npc) = obj.npc_type.as_deref() else {
                continue;
            };
            match npc {
                "keeper" => keeper_ai(world, room_name, obj, &mut entries),
                "invader" => invader_ai(world, room_name, obj, &mut entries),
                _ => {}
            }
        }
    }
    IntentList { entries }
}

/// keeper AI: 守卫 source —— 玩家 creep 进入仇恨范围就打 (不追击), 否则回防到守卫圈。
fn keeper_ai(
    world: &World,
    room_name: &str,
    keeper: &Obj,
    entries: &mut Vec<(String, String, Value)>,
) {
    let room = &world.rooms[room_name];
    let id = World::id_string(keeper.id);

    // 1. 仇恨: 最近的玩家 creep (chebyshev ≤ KEEPER_AGGRO_RANGE)。BTreeMap 即 id 序,
    //    同距取 id 小者, 确定性。keeper 不追击: 目标离开范围就回防。
    let mut target: Option<(u64, u8, u8)> = None;
    for o in room.objects.values() {
        if o.obj_type != "creep" || !o.mine {
            continue;
        }
        let dist = (o.x as i32 - keeper.x as i32).abs().max((o.y as i32 - keeper.y as i32).abs());
        if dist <= constants::KEEPER_AGGRO_RANGE {
            target = Some((o.id, o.x, o.y));
            break;
        }
    }

    if let Some((tid, _, _)) = target {
        let t = &room.objects[&tid];
        let dist = (t.x as i32 - keeper.x as i32).abs().max((t.y as i32 - keeper.y as i32).abs());
        entries.push((id.clone(), "attack".into(), json!({ "id": World::id_string(tid) })));
        if dist > 1 && keeper_has_part(keeper, "ranged_attack") {
            entries.push((id, "rangedAttack".into(), json!({ "id": World::id_string(tid) })));
        }
        return;
    }

    // 2. 无仇恨: 回防守卫圈 (距被守卫 source > KEEPER_GUARD_RANGE 才走)
    let Some(src_id) = keeper.guard_source else {
        return;
    };
    let Some(src) = room.objects.get(&src_id) else {
        return;
    };
    let dist = (src.x as i32 - keeper.x as i32).abs().max((src.y as i32 - keeper.y as i32).abs());
    if dist > constants::KEEPER_GUARD_RANGE {
        if let Some(dir) = step_direction(keeper.x, keeper.y, src.x, src.y) {
            entries.push((id, "move".into(), json!({ "direction": dir })));
        }
    }
}

/// invader AI: 搜索最近的玩家 creep/结构, 近战攻击或远程攻击, 否则移动接近。
fn invader_ai(
    world: &World,
    room_name: &str,
    invader: &Obj,
    entries: &mut Vec<(String, String, Value)>,
) {
    let room = &world.rooms[room_name];
    let id = World::id_string(invader.id);

    // 目标: 最近的玩家对象 (creep 优先于结构, 同距取 id 小者 = 确定性)。
    // 控制器与掉落资源不是袭击目标 (官方 invader 打 creep/结构)。
    let mut best: Option<(u64, u8, u8, i32)> = None;
    for o in room.objects.values() {
        if !o.mine || o.obj_type == "energy" || o.obj_type == "controller" {
            continue;
        }
        let is_creep = o.obj_type == "creep" && !o.spawning;
        let is_structure =
            constants::is_owned_structure(&o.obj_type) || o.obj_type == "constructionSite";
        if !is_creep && !is_structure {
            continue;
        }
        let dist = (o.x as i32 - invader.x as i32)
            .abs()
            .max((o.y as i32 - invader.y as i32).abs());
        let better = match best {
            None => true,
            Some((_, _, _, bd)) => dist < bd,
        };
        if better {
            best = Some((o.id, o.x, o.y, dist));
        }
    }
    let Some((tid, tx, ty, dist)) = best else {
        return; // 房间内没有玩家对象: 原地待命
    };

    if dist <= 1 {
        entries.push((id, "attack".into(), json!({ "id": World::id_string(tid) })));
    } else if dist <= 3 && invader_has_part(invader, "ranged_attack") {
        entries.push((id, "rangedAttack".into(), json!({ "id": World::id_string(tid) })));
    } else if let Some(dir) = step_direction(invader.x, invader.y, tx, ty) {
        entries.push((id, "move".into(), json!({ "direction": dir })));
    }
}

/// tick 末: powerBank 线性衰减; invader 到达 lifetime 撤离 (官方"离开并消失")。
pub fn run_npc_tick(world: &mut World) {
    let tick = world.tick;
    let room_names: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names {
        let Some(room) = world.rooms.get_mut(room_name) else {
            continue;
        };

        // invader 撤离 (不留墓碑 —— 官方只是离开地图; <= 兜底漏 tick 也不会常驻)
        let expire: Vec<u64> = room
            .objects
            .values()
            .filter(|o| {
                o.npc_type.as_deref() == Some("invader")
                    && o.npc_despawn_time.is_some_and(|t| t <= tick)
            })
            .map(|o| o.id)
            .collect();
        for id in expire {
            room.objects.remove(&id);
        }

        // powerBank 线性衰减: 到点按比例削 hits/power, 归零即消失
        let decay_ids: Vec<u64> = room
            .objects
            .values()
            .filter(|o| {
                o.obj_type == "powerBank"
                    && o.next_decay_time.is_some_and(|t| t <= tick)
            })
            .map(|o| o.id)
            .collect();
        for id in decay_ids {
            let bank = room.objects.get_mut(&id).unwrap();
            let per_step = (bank.hits_max as f64
                * constants::POWER_BANK_DECAY_STEP as f64
                / constants::POWER_BANK_DECAY as f64)
                .ceil() as u32;
            bank.hits = bank.hits.saturating_sub(per_step.max(1));
            // power 与 hits 同比衰减 (近似: 官方未公开 power 衰减曲线, 取线性)
            if let Some(power) = bank.store.get_mut("power") {
                let power_per_step = (constants::POWER_BANK_CAPACITY_MAX as f64
                    * constants::POWER_BANK_DECAY_STEP as f64
                    / constants::POWER_BANK_DECAY as f64)
                    .ceil() as u32;
                *power = power.saturating_sub(power_per_step.max(1));
            }
            if bank.hits == 0 {
                room.objects.remove(&id);
                room.push_event(
                    crate::engine::intents::EVENT_OBJECT_DESTROYED,
                    id,
                    json!({ "type": "powerBank" }),
                );
            } else {
                bank.next_decay_time = Some(tick + constants::POWER_BANK_DECAY_STEP);
            }
        }
    }

    // keeper 生成 (lair 到期; 受扰倒计时由意图结算里的攻击重置)
    let tick2 = world.tick;
    let room_names2: Vec<String> = world.rooms.keys().cloned().collect();
    for room_name in &room_names2 {
        let due: Vec<u64> = {
            let Some(room) = world.rooms.get(room_name) else {
                continue;
            };
            room.objects
                .values()
                .filter(|o| {
                    o.obj_type == "keeperLair"
                        && o.next_spawn_time.is_some_and(|t| t <= tick2)
                })
                .map(|o| o.id)
                .collect()
        };
        if due.is_empty() {
            continue;
        }
        for lair_id in due {
            spawn_keepers(world, room_name, lair_id);
        }
    }

    // invader 侵袭波次 (场景 npcs.invaders 开启时; 先取配置再借 world, 避免借用冲突)
    if let Some(cfg) = world.npc_invaders.clone() {
        if tick >= cfg.first && (tick - cfg.first) % cfg.every == 0 {
            spawn_invader_wave(world, &cfg);
        }
    }
}

/// lair 到期: 旧 keeper 自杀, 生成新 keeper 小队 (2 个, 出生在 lair 邻格)。
/// 只更替本 lair 的 keeper (guard_lair 绑定), 同房间其他 lair 的守卫不受影响。
fn spawn_keepers(world: &mut World, room_name: &str, lair_id: u64) {
    // 旧 keeper 自杀 (wiki: 新 keeper 生成时旧的仍在 → 旧的自杀)
    let old_keepers: Vec<u64> = {
        let Some(room) = world.rooms.get(room_name) else {
            return;
        };
        room.objects
            .values()
            .filter(|o| o.npc_type.as_deref() == Some("keeper") && o.guard_lair == Some(lair_id))
            .map(|o| o.id)
            .collect()
    };
    for id in old_keepers {
        // 注意: 本机 rustc 的 IndexMut 索引表达式不可用, 统一走 get_mut
        if let Some(room) = world.rooms.get_mut(room_name) {
            room.objects.remove(&id);
        }
    }

    let (lx, ly, guard) = {
        let Some(room) = world.rooms.get(room_name) else {
            return;
        };
        let lair = &room.objects[&lair_id];
        (lair.x, lair.y, lair.guard_source)
    };
    let next = world.tick + constants::KEEPER_SPAWN_INTERVAL;
    if let Some(room) = world.rooms.get_mut(room_name) {
        if let Some(lair) = room.objects.get_mut(&lair_id) {
            lair.next_spawn_time = Some(next);
        }
    }

    for i in 0..2usize {
        let (kx, ky) = spawn_slot(world, room_name, lx, ly, i);
        let id = alloc_id(&mut world.next_id);
        let body = keeper_body();
        let mut keeper = creep_obj(id, &format!("Keeper{id}"), kx, ky, body);
        keeper.mine = false;
        keeper.enemy = true;
        keeper.npc_type = Some("keeper".into());
        keeper.guard_source = guard;
        keeper.guard_lair = Some(lair_id);
        // ticksToLive 展示值: NPC creep 不走 tick_creeps 的自然寿命/墓碑, 存亡由本模块管理
        keeper.age_time = Some(world.tick + constants::CREEP_LIFE_TIME);
        if let Some(room) = world.rooms.get_mut(room_name) {
            room.objects.insert(id, keeper);
        }
    }
}

/// invader 侵袭波: 从玩家房间边缘生成 `squad` 个 invader (官方: 侵袭者从房间边界进入地图)。
fn spawn_invader_wave(world: &mut World, cfg: &InvaderConfig) {
    // 目标房间: 有玩家对象的房间 (确定性: 房名序第一个)
    let room_name = world
        .rooms
        .iter()
        .find(|(_, room)| {
            room.objects
                .values()
                .any(|o| o.mine && (o.obj_type == "creep" || constants::is_owned_structure(&o.obj_type)))
        })
        .map(|(name, _)| name.clone());
    let Some(room_name) = room_name else {
        return;
    };

    let despawn = world.tick + cfg.lifetime;
    for i in 0..cfg.squad {
        let (ix, iy) = invader_entry_tile(world, &room_name, i);
        let id = alloc_id(&mut world.next_id);
        let body = invader_body();
        let mut invader = creep_obj(id, &format!("Invader{id}"), ix, iy, body);
        invader.mine = false;
        invader.enemy = true;
        invader.npc_type = Some("invader".into());
        invader.npc_despawn_time = Some(despawn);
        // ticksToLive 展示值; 到期撤离由 npc_despawn_time 驱动 (不走自然寿命/墓碑)
        invader.age_time = Some(despawn);
        if let Some(room) = world.rooms.get_mut(&room_name) {
            room.objects.insert(id, invader);
        }
    }
}

/// invader 进入点: 房间四边中点轮换, 内缩 1 格 (出口格会立即触发越房)。
/// 同批第 k 轮 (i/4) 沿该边平移 3 格展开; 该格是墙/已被占用时退回邻格搜索。
fn invader_entry_tile(world: &World, room_name: &str, i: usize) -> (u8, u8) {
    let k = (i / 4) as u8;
    let base = match i % 4 {
        0 => (25u8.saturating_add(k.saturating_mul(3)).min(48), 1u8), // TOP
        1 => (48u8, 25u8.saturating_add(k.saturating_mul(3)).min(48)), // RIGHT
        2 => (25u8.saturating_sub(k.saturating_mul(3)).max(1), 48u8), // BOTTOM
        _ => (1u8, 25u8.saturating_sub(k.saturating_mul(3)).max(1)),  // LEFT
    };
    let room = &world.rooms[room_name];
    let blocked = room.terrain.is_wall(base.0 as usize, base.1 as usize)
        || room.objects.values().any(|o| o.x == base.0 && o.y == base.1);
    if blocked {
        spawn_slot(world, room_name, base.0, base.1, i)
    } else {
        base
    }
}

fn keeper_body() -> Vec<BodyPart> {
    let mut body = Vec::new();
    for _ in 0..constants::KEEPER_BODY_MOVE {
        body.push(BodyPart::new("move"));
    }
    for _ in 0..constants::KEEPER_BODY_ATTACK {
        body.push(BodyPart::new("attack"));
    }
    for _ in 0..constants::KEEPER_BODY_RANGED_ATTACK {
        body.push(BodyPart::new("ranged_attack"));
    }
    body
}

fn invader_body() -> Vec<BodyPart> {
    let mut body = Vec::new();
    for _ in 0..constants::INVADER_BODY_MOVE {
        body.push(BodyPart::new("move"));
    }
    for _ in 0..constants::INVADER_BODY_ATTACK {
        body.push(BodyPart::new("attack"));
    }
    for _ in 0..constants::INVADER_BODY_RANGED_ATTACK {
        body.push(BodyPart::new("ranged_attack"));
    }
    body
}

fn keeper_has_part(keeper: &Obj, part: &str) -> bool {
    keeper.body.iter().any(|p| p.part_type == part && p.hits > 0)
}

fn invader_has_part(invader: &Obj, part: &str) -> bool {
    invader.body.iter().any(|p| p.part_type == part && p.hits > 0)
}

/// 朝目标的下一步方向 (官方方向常量; 斜向优先)。不可达/已到达返回 None。
fn step_direction(fx: u8, fy: u8, tx: u8, ty: u8) -> Option<i32> {
    let dx = tx as i32 - fx as i32;
    let dy = ty as i32 - fy as i32;
    let sx = dx.signum();
    let sy = dy.signum();
    if sx == 0 && sy == 0 {
        return None;
    }
    let dir = match (sx, sy) {
        (0, -1) => 1,        // TOP
        (1, -1) => 2,        // TOP_RIGHT
        (1, 0) => 3,         // RIGHT
        (1, 1) => 4,         // BOTTOM_RIGHT
        (0, 1) => 5,         // BOTTOM
        (-1, 1) => 6,        // BOTTOM_LEFT
        (-1, 0) => 7,        // LEFT
        (-1, -1) => 8,       // TOP_LEFT
        _ => return None,
    };
    Some(dir)
}

/// 在 (cx, cy) 周围找一个非墙空格落位; 第 i 个生成者沿螺旋外扩。
/// 优先落在房间内部 (1..=48): 官方 creep 不能停留在出口格 (落上去会立即越房)。
fn spawn_slot(world: &World, room_name: &str, cx: u8, cy: u8, i: usize) -> (u8, u8) {
    let room = &world.rooms[room_name];
    let candidates_in = |lo: i32, hi: i32| -> Vec<(u8, u8)> {
        (0..8i32)
            .map(|d| dir_delta(d + 1))
            .filter(|(dx, dy)| {
                let x = cx as i32 + dx;
                let y = cy as i32 + dy;
                (lo..hi).contains(&x) && (lo..hi).contains(&y)
            })
            .map(|(dx, dy)| ((cx as i32 + dx) as u8, (cy as i32 + dy) as u8))
            .collect()
    };
    let interior = candidates_in(1, 49);
    let candidates = if interior.is_empty() { candidates_in(0, 50) } else { interior };
    let mut idx = i % candidates.len().max(1);
    for _ in 0..candidates.len() {
        let (x, y) = candidates[idx % candidates.len()];
        if !room.terrain.is_wall(x as usize, y as usize) {
            return (x, y);
        }
        idx += 1;
    }
    (cx, cy)
}

/// 官方方向常量 → (dx, dy)
fn dir_delta(dir: i32) -> (i32, i32) {
    match dir {
        1 => (0, -1),
        2 => (1, -1),
        3 => (1, 0),
        4 => (1, 1),
        5 => (0, 1),
        6 => (-1, 1),
        7 => (-1, 0),
        8 => (-1, -1),
        _ => (0, 0),
    }
}
