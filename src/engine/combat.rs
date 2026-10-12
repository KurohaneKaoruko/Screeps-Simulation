//! 战斗机制: 安全模式 (safe mode) + 核弹 (nuke)。
//!
//! 官方语义:
//! - 安全模式: `controller.activateSafeMode()` 激活, 持续 `SAFE_MODE_DURATION` (20000) tick,
//!   两次激活间隔 `SAFE_MODE_COOLDOWN` (50000) tick; 一个 shard 同时只能有一个房间处于安全模式;
//!   激活期间非房主对该房间对象的攻击一律被拦截 (房主自己的攻击不受影响)。
//! - 核弹: `nuker.launchNuke(pos)` 消耗满仓 energy(300000) + ghodium(5000), 冷却 `NUKER_COOLDOWN`;
//!   飞行 `NUKE_LAND_TIME` (50000) tick 后落地: 移除全房间 creep / 工地 / 掉落物 / 墓碑 / 废墟,
//!   落点造成 `NUKE_DAMAGE[0]` (1e7) 伤害、5×5 内 `NUKE_DAMAGE[2]` (5e6) 伤害 (无视 rampart),
//!   取消该房间安全模式并把冷却清零, 控制器进入 `upgradeBlocked` 200 tick。

use serde_json::json;

use crate::engine::constants;
use crate::engine::world::World;

/// 一颗在飞的核弹 (官方 Nuke)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Nuke {
    pub id: u64,
    pub launch_room: String,
    pub target_room: String,
    pub x: u8,
    pub y: u8,
    /// 落地 (impact) tick
    pub land_tick: u64,
}

/// 本 tick 登记的核弹发射 (官方 nuker.launchNuke 意图)
#[derive(Clone, Debug)]
pub struct NukeLaunch {
    pub room: String,
    pub nuker: u64,
    pub x: u8,
    pub y: u8,
    pub target_room: String,
}

/// 房间是否处于安全模式 (本房间控制器归我且未到期)
pub fn safe_mode_active(world: &World, room_name: &str) -> bool {
    world
        .rooms
        .get(room_name)
        .and_then(|r| r.controller())
        .is_some_and(|c| c.mine && c.safe_mode_until > world.tick)
}

/// 除 `except` 外, 本玩家是否已有房间处于安全模式 (官方: 一个 shard 同时只能一个)
fn other_room_in_safe_mode(world: &World, except: &str) -> bool {
    world.rooms.iter().any(|(name, r)| {
        name != except
            && r.controller().is_some_and(|c| c.mine && c.safe_mode_until > world.tick)
    })
}

/// 激活安全模式 (官方 controller.activateSafeMode): 返回是否成功。
/// 失败原因 (未拥有 / 已激活 / 无可用次数 / 冷却中 / 被 upgradeBlocked / 别的房间已开) 均返回 false。
pub fn activate_safe_mode(world: &mut World, controller_id: u64) -> bool {
    let tick = world.tick;
    let Some(room_name) = world
        .rooms
        .iter()
        .find(|(_, r)| r.objects.contains_key(&controller_id))
        .map(|(n, _)| n.clone())
    else {
        return false;
    };
    let (is_controller, mine, active, available, cooldown, blocked) = {
        let c = &world.rooms[&room_name].objects[&controller_id];
        (
            c.obj_type == "controller",
            c.mine,
            c.safe_mode_until > tick,
            c.safe_mode_available,
            c.safe_mode_cooldown_until,
            c.upgrade_blocked,
        )
    };
    if !is_controller || !mine || active || available == 0 || cooldown > tick || blocked > tick {
        return false;
    }
    if other_room_in_safe_mode(world, &room_name) {
        return false;
    }
    let c = world
        .rooms
        .get_mut(&room_name)
        .unwrap()
        .objects
        .get_mut(&controller_id)
        .unwrap();
    c.safe_mode_until = tick + constants::SAFE_MODE_DURATION;
    c.safe_mode_available -= 1;
    c.safe_mode_cooldown_until = tick + constants::SAFE_MODE_COOLDOWN;
    true
}

/// 结算本 tick 登记的核弹发射 (官方 nuker/launch-nuke.js)
pub fn execute_nuke_launches(world: &mut World, launches: &[NukeLaunch]) {
    for l in launches {
        let tick = world.tick;
        let Some(n) = world.rooms.get(&l.room).and_then(|r| r.objects.get(&l.nuker)) else {
            continue;
        };
        if !n.mine || !n.is_active() || n.cooldown_time > tick {
            continue;
        }
        let energy = n.store.get("energy").copied().unwrap_or(0);
        let ghodium = n.store.get("G").copied().unwrap_or(0);
        if energy < constants::NUKER_ENERGY_CAPACITY || ghodium < constants::NUKER_GHODIUM_CAPACITY {
            continue;
        }
        {
            let n = world
                .rooms
                .get_mut(&l.room)
                .unwrap()
                .objects
                .get_mut(&l.nuker)
                .unwrap();
            n.store
                .insert("energy".into(), energy - constants::NUKER_ENERGY_CAPACITY);
            n.store.insert("G".into(), ghodium - constants::NUKER_GHODIUM_CAPACITY);
            n.cooldown_time = tick + constants::NUKER_COOLDOWN;
        }
        let id = world.alloc_id();
        world.nukes.push(Nuke {
            id,
            launch_room: l.room.clone(),
            target_room: l.target_room.clone(),
            x: l.x,
            y: l.y,
            land_tick: tick + constants::NUKE_LAND_TIME,
        });
    }
}

/// 核弹相位: 处理本 tick 落地的核弹
pub fn run_nuke_tick(world: &mut World) {
    if world.nukes.is_empty() {
        return;
    }
    let tick = world.tick;
    let landing: Vec<Nuke> = world.nukes.iter().filter(|n| n.land_tick <= tick).cloned().collect();
    if landing.is_empty() {
        return;
    }
    world.nukes.retain(|n| n.land_tick > tick);
    for nuke in landing {
        impact(world, &nuke);
    }
}

/// 核弹落地效果
fn impact(world: &mut World, nuke: &Nuke) {
    let tick = world.tick;
    let room_name = nuke.target_room.clone();
    let Some(room) = world.rooms.get_mut(&room_name) else { return };

    // 1) 立即移除全房间 creep / 工地 / 掉落资源 / 墓碑 / 废墟 (无视 rampart)
    let clear: Vec<u64> = room
        .objects
        .values()
        .filter(|o| {
            matches!(
                o.obj_type.as_str(),
                "creep" | "constructionSite" | "energy" | "tombstone" | "ruin"
            )
        })
        .map(|o| o.id)
        .collect();
    for id in clear {
        room.objects.remove(&id);
    }

    // 2) 结构伤害: 落点 NUKE_DAMAGE[0], 5×5 内 NUKE_DAMAGE[2] (无视 rampart)
    let mut destroyed: Vec<u64> = Vec::new();
    for o in room.objects.values_mut() {
        if o.obj_type == "controller" || o.obj_type == "portal" {
            continue;
        }
        if !constants::is_known_structure_type(&o.obj_type) {
            continue;
        }
        let d = (o.x as i32 - nuke.x as i32).abs().max((o.y as i32 - nuke.y as i32).abs());
        let dmg = if d == 0 {
            constants::NUKE_DAMAGE_RANGE_0
        } else if d <= 2 {
            constants::NUKE_DAMAGE_RANGE_2
        } else {
            0
        };
        if dmg == 0 {
            continue;
        }
        o.hits = o.hits.saturating_sub(dmg);
        if o.hits == 0 {
            destroyed.push(o.id);
        }
    }
    for id in &destroyed {
        let type_name = room.objects.get(id).map(|o| o.obj_type.clone()).unwrap_or_default();
        room.objects.remove(id);
        room.push_event(
            crate::engine::intents::EVENT_OBJECT_DESTROYED,
            *id,
            json!({ "type": type_name }),
        );
    }

    // 3) 安全模式被取消, 冷却清零; 控制器 upgradeBlocked 200 tick
    if let Some(c) = room.objects.values_mut().find(|o| o.obj_type == "controller") {
        c.safe_mode_until = 0;
        c.safe_mode_cooldown_until = 0;
        c.upgrade_blocked = tick + constants::CONTROLLER_NUKE_BLOCKED_UPGRADE;
    }
}
