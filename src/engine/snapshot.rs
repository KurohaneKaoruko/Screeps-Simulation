//! 可见世界快照序列化: Rust 世界状态 → JSON (JS shim 据此重建 Game 全局)。
//! 快照即事实, bot 侧一切读取都以快照为准 —— 与真实游戏"每 tick 重放快照"的模型一致。

use crate::engine::constants;
use crate::engine::objects::{Obj, Store};
use crate::engine::world::World;
use serde_json::{json, Map, Value};

pub struct CpuView {
    pub limit: f64,
    pub tick_limit: f64,
    pub bucket: f64,
}

pub struct SnapshotOptions {
    pub cpu: CpuView,
    /// 调试: 无视野限制, 全房间可见
    pub always_visible: bool,
}

pub fn build_snapshot(world: &World, opts: &SnapshotOptions) -> Value {
    let mut terrain = Map::new();
    let mut room_names = Vec::new();
    for (name, room) in &world.rooms {
        room_names.push(name.clone());
        terrain.insert(name.clone(), Value::String(room.terrain.to_string()));
    }

    let mut rooms = Map::new();
    for (name, room) in &world.rooms {
        if !opts.always_visible && !room.visible() {
            continue;
        }
        let controller_id = room
            .controller()
            .map(|c| World::id_string(c.id));
        let storage_id = room
            .objects
            .values()
            .find(|o| o.obj_type == "storage" && o.mine)
            .map(|o| World::id_string(o.id));
        let mut objects = Vec::with_capacity(room.objects.len());
        for obj in room.objects.values() {
            objects.push(object_json(obj, world));
        }
        let events: Vec<Value> = room
            .events
            .iter()
            .map(|e| {
                json!({
                    "event": e.event,
                    "objectId": World::id_string(e.object_id),
                    "data": e.data,
                })
            })
            .collect();
        rooms.insert(
            name.clone(),
            json!({
                "energyAvailable": room.energy_available(),
                "energyCapacityAvailable": room.energy_capacity_available(),
                "controller": controller_id,
                "storage": storage_id,
                "objects": objects,
                "events": events,
            }),
        );
    }

    let (gcl_progress, gcl_total) = world.gcl_level_progress();
    json!({
        "time": world.tick,
        "cpu": {
            "limit": opts.cpu.limit,
            "tickLimit": opts.cpu.tick_limit,
            "bucket": opts.cpu.bucket,
        },
        "gcl": {
            "level": world.gcl_level(),
            "progress": gcl_progress,
            "progressTotal": gcl_total,
        },
        "terrain": Value::Object(terrain),
        "roomNames": room_names,
        "rooms": Value::Object(rooms),
    })
}

fn object_json(obj: &Obj, world: &World) -> Value {
    let id = World::id_string(obj.id);
    let base = |extra: Map<String, Value>| {
        let mut map = Map::new();
        map.insert("id".into(), json!(id));
        map.insert("type".into(), json!(obj.obj_type));
        map.insert("x".into(), json!(obj.x));
        map.insert("y".into(), json!(obj.y));
        for (k, v) in extra {
            map.insert(k, v);
        }
        Value::Object(map)
    };

    match obj.obj_type.as_str() {
        "creep" => {
            let mut extra = Map::new();
            extra.insert("name".into(), json!(obj.name.clone().unwrap_or_default()));
            extra.insert("my".into(), json!(obj.mine));
            extra.insert(
                "body".into(),
                Value::Array(
                    obj.body
                        .iter()
                        .map(|p| json!({"type": p.part_type, "hits": p.hits}))
                        .collect(),
                ),
            );
            extra.insert("store".into(), store_json(&obj.store));
            extra.insert("storeCapacity".into(), json!(obj.store_capacity()));
            extra.insert("fatigue".into(), json!(obj.fatigue));
            if obj.spawning {
                extra.insert("spawning".into(), json!(true));
            } else {
                extra.insert(
                    "ticksToLive".into(),
                    json!(obj.age_time.map(|t| t.saturating_sub(world.tick))),
                );
            }
            extra.insert("hits".into(), json!(obj.hits));
            extra.insert("hitsMax".into(), json!(obj.hits_max));
            base(extra)
        }
        "spawn" => {
            let mut extra = structure_common(obj);
            extra.insert("name".into(), json!(obj.name.clone().unwrap_or_default()));
            extra.insert(
                "energy".into(),
                json!(obj.store.get("energy").copied().unwrap_or(0)),
            );
            extra.insert("energyCapacity".into(), json!(obj.energy_capacity));
            match &obj.spawn_job {
                Some(job) => {
                    extra.insert(
                        "spawning".into(),
                        json!({
                            "name": job.name,
                            "needTime": job.need_time,
                            "remainingTime": job.spawn_time.saturating_sub(world.tick),
                        }),
                    );
                }
                None => {
                    extra.insert("spawning".into(), Value::Null);
                }
            }
            base(extra)
        }
        "extension" | "storage" | "container" | "tower" | "terminal" | "link" | "lab"
        | "powerSpawn" | "factory" | "nuker" => {
            let mut extra = structure_common(obj);
            if obj.obj_type == "extension" || obj.obj_type == "tower" || obj.obj_type == "link" {
                let energy = obj.store.get("energy").copied().unwrap_or(0);
                extra.insert("energy".into(), json!(energy));
                extra.insert("energyCapacity".into(), json!(obj.energy_capacity));
            }
            if obj.obj_type == "link" {
                extra.insert("cooldown".into(), json!(obj.cooldown));
            }
            if obj.obj_type == "container" || obj.obj_type == "storage" {
                if let Some(t) = obj.next_decay_time {
                    extra.insert("ticksToDecay".into(), json!(t.saturating_sub(world.tick)));
                }
            }
            base(extra)
        }
        "road" | "constructedWall" | "rampart" => {
            let mut extra = Map::new();
            extra.insert("structureType".into(), json!(obj.obj_type));
            extra.insert("hits".into(), json!(obj.hits));
            extra.insert("hitsMax".into(), json!(obj.hits_max));
            if obj.obj_type == "rampart" {
                extra.insert("isPublic".into(), json!(false));
                if let Some(t) = obj.next_decay_time {
                    extra.insert("ticksToDecay".into(), json!(t.saturating_sub(world.tick)));
                }
            }
            base(extra)
        }
        "source" => {
            let mut extra = Map::new();
            let energy = obj.store.get("energy").copied().unwrap_or(0);
            extra.insert("energy".into(), json!(energy));
            extra.insert("energyCapacity".into(), json!(obj.energy_capacity));
            extra.insert(
                "ticksToRegeneration".into(),
                json!(obj.next_regeneration_time.map(|t| t.saturating_sub(world.tick))),
            );
            base(extra)
        }
        "mineral" => {
            let mut extra = Map::new();
            extra.insert(
                "mineralType".into(),
                json!(obj.mineral_type.clone().unwrap_or_else(|| "H".into())),
            );
            extra.insert("mineralAmount".into(), json!(obj.mineral_amount.unwrap_or(0)));
            base(extra)
        }
        "controller" => {
            let mut extra = Map::new();
            extra.insert("my".into(), json!(obj.mine));
            extra.insert("level".into(), json!(obj.level));
            extra.insert("progress".into(), json!(obj.progress));
            let total = constants::CONTROLLER_LEVELS
                .iter()
                .find(|(l, _)| *l == obj.level)
                .map(|(_, p)| *p)
                .unwrap_or(0);
            extra.insert("progressTotal".into(), json!(total));
            extra.insert(
                "ticksToDowngrade".into(),
                json!(obj.downgrade_time.map(|t| t.saturating_sub(world.tick))),
            );
            extra.insert("safeModeAvailable".into(), json!(0));
            extra.insert("safeMode".into(), Value::Null);
            extra.insert(
                "upgradeBlocked".into(),
                json!(obj.upgrade_blocked.saturating_sub(world.tick)),
            );
            extra.insert(
                "reservation".into(),
                match obj.reservation_end_time {
                    Some(end) => json!({
                        "username": "Nexus",
                        "ticksToEndTime": end.saturating_sub(world.tick),
                    }),
                    None => Value::Null,
                },
            );
            extra.insert(
                "sign".into(),
                match &obj.sign {
                    Some(text) => json!({ "username": "Nexus", "text": text, "time": 0, "datetime": 0 }),
                    None => Value::Null,
                },
            );
            base(extra)
        }
        "constructionSite" => {
            let mut extra = Map::new();
            extra.insert(
                "structureType".into(),
                json!(obj.structure_type.clone().unwrap_or_default()),
            );
            extra.insert("progress".into(), json!(obj.progress));
            extra.insert("progressTotal".into(), json!(obj.progress_total));
            extra.insert("my".into(), json!(obj.mine));
            base(extra)
        }
        "energy" => {
            let mut extra = Map::new();
            extra.insert(
                "energy".into(),
                json!(obj.store.get("energy").copied().unwrap_or(0)),
            );
            extra.insert("resourceType".into(), json!("energy"));
            base(extra)
        }
        "tombstone" => {
            let mut extra = Map::new();
            extra.insert("store".into(), store_json(&obj.store));
            if let Some(t) = obj.next_decay_time {
                extra.insert("ticksToDecay".into(), json!(t.saturating_sub(world.tick)));
            }
            extra.insert("creepName".into(), json!(obj.creep_name.clone()));
            base(extra)
        }
        other => {
            let _ = other;
            base(Map::new())
        }
    }
}

fn structure_common(obj: &Obj) -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert("structureType".into(), json!(obj.obj_type));
    extra.insert("hits".into(), json!(obj.hits));
    extra.insert("hitsMax".into(), json!(obj.hits_max));
    extra.insert("my".into(), json!(obj.mine));
    extra.insert("store".into(), store_json(&obj.store));
    // 超出 RCL 上限被停用的 spawn/extension (官方 isActive() 为 false)
    extra.insert("off".into(), json!(obj.off));
    extra.insert(
        "storeCapacity".into(),
        json!(if obj.obj_type == "extension" || obj.obj_type == "spawn" {
            obj.energy_capacity
        } else {
            obj.store_capacity()
        }),
    );
    extra
}

/// 官方 Store 只包含非零资源; 缺失键由 shim 的 Proxy 按 RESOURCES_ALL 返回 0
fn store_json(store: &Store) -> Value {
    let mut map = Map::new();
    for (k, v) in store {
        if *v > 0 {
            map.insert(k.clone(), json!(v));
        }
    }
    Value::Object(map)
}
