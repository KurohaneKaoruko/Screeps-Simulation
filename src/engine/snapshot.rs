//! 可见世界快照序列化: Rust 世界状态 → JSON (JS shim 据此重建 Game 全局)。//! 快照即事实, bot 侧一切读取都以快照为准 —— 与真实游戏"每 tick 重放快照"的模型一致。
//!
//! 两条路径:
//! - `build_snapshot` 直接写 String (热路径, 不建 serde_json::Value 树)
//! - `build_snapshot_value` 旧的 Value 版 (对拍基准: parity 测试保证两者语义一致)
//!
//! terrain 不可变: 只在 isolate 新建/重置后的首个快照携带 (`include_terrain`),
//! shim 的 terrainCache 解析一次后常驻, 后续 tick 不再重发 20KB/房。

use crate::engine::constants;
use crate::engine::objects::{Obj, Store};
use crate::engine::world::World;
use serde_json::{json, Map, Value};
use std::fmt::Write as _;

pub struct CpuView {
    pub limit: f64,
    pub tick_limit: f64,
    pub bucket: f64,
}

pub struct SnapshotOptions {
    pub cpu: CpuView,
    /// 调试: 无视野限制, 全房间可见
    pub always_visible: bool,
    /// 是否携带地形 (仅 isolate 新建/重置后的首个快照需要; terrain 不可变)
    pub include_terrain: bool,
}

// ── 直接序列化 (热路径) ───────────────────────────────────

/// f64 按 serde_json (ryu 最短表示) 写出, 保证与 Value 版对拍一致
fn put_f64(out: &mut String, v: f64) {
    let _ = write!(out, "{}", json!(v));
}

/// 字符串按 JSON 转义规则写出 (名字/文本可能含任意字符)
fn put_str(out: &mut String, s: &str) {
    let _ = write!(out, "{}", json!(s));
}

pub fn build_snapshot(world: &World, opts: &SnapshotOptions) -> String {
    let mut out = String::with_capacity(1024 + world.rooms.len() * 96);
    out.push_str("{\"time\":");
    let _ = write!(out, "{}", world.tick);
    out.push_str(",\"cpu\":{\"limit\":");
    put_f64(&mut out, opts.cpu.limit);
    out.push_str(",\"tickLimit\":");
    put_f64(&mut out, opts.cpu.tick_limit);
    out.push_str(",\"bucket\":");
    put_f64(&mut out, opts.cpu.bucket);
    out.push('}');
    let (gcl_progress, gcl_total) = world.gcl_level_progress();
    out.push_str(",\"gcl\":{\"level\":");
    let _ = write!(out, "{}", world.gcl_level());
    out.push_str(",\"progress\":");
    let _ = write!(out, "{}", gcl_progress);
    out.push_str(",\"progressTotal\":");
    let _ = write!(out, "{}", gcl_total);
    out.push('}');

    // roomNames: 全部房间 (地图 API 的世界清单)
    out.push_str(",\"roomNames\":[");
    for (i, name) in world.rooms.keys().enumerate() {
        if i > 0 {
            out.push(',');
        }
        put_str(&mut out, name);
    }
    out.push(']');

    // terrain: 不可变, 仅首个快照携带
    out.push_str(",\"terrain\":");
    if opts.include_terrain {
        out.push('{');
        for (i, (name, room)) in world.rooms.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            put_str(&mut out, name);
            out.push(':');
            put_str(&mut out, &room.terrain_json);
        }
        out.push('}');
    } else {
        out.push_str("{}");
    }

    // rooms: 可见房间的对象与事件
    out.push_str(",\"rooms\":{");
    let mut first_room = true;
    for (name, room) in &world.rooms {
        if !opts.always_visible && !room.visible() {
            continue;
        }
        if !first_room {
            out.push(',');
        }
        first_room = false;
        put_str(&mut out, name);
        out.push_str(":{\"energyAvailable\":");
        let _ = write!(out, "{}", room.energy_available());
        out.push_str(",\"energyCapacityAvailable\":");
        let _ = write!(out, "{}", room.energy_capacity_available());
        out.push_str(",\"controller\":");
        match room.controller() {
            Some(c) => {
                out.push('"');
                let _ = write!(out, "{:024x}", c.id);
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"storage\":");
        match room.objects.values().find(|o| o.obj_type == "storage" && o.mine) {
            Some(s) => {
                out.push('"');
                let _ = write!(out, "{:024x}", s.id);
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"objects\":[");
        for (i, obj) in room.objects.values().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_object(&mut out, obj, world);
        }
        out.push_str("],\"events\":[");
        for (i, e) in room.events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"event\":");
            let _ = write!(out, "{}", e.event);
            out.push_str(",\"objectId\":\"");
            let _ = write!(out, "{:024x}", e.object_id);
            out.push_str("\",\"data\":");
            out.push_str(&e.data.to_string());
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push('}');

    // constructionSites: 无视野房间里自己的工地 (官方 userObjects; 工地不提供视野,
    // 但仍在 Game.constructionSites / getObjectById 中, .room 为 undefined)
    out.push_str(",\"constructionSites\":[");
    let mut first_site = true;
    for (name, room) in &world.rooms {
        if opts.always_visible || room.visible() {
            continue;
        }
        for obj in room.objects.values() {
            if obj.obj_type != "constructionSite" || !obj.mine {
                continue;
            }
            if !first_site {
                out.push(',');
            }
            first_site = false;
            let mut tmp = String::new();
            write_object(&mut tmp, obj, world);
            out.push_str("{\"room\":");
            put_str(&mut out, name);
            out.push(',');
            out.push_str(&tmp[1..]);
        }
    }
    out.push(']');

    // flags: 全部旗 (与视野无关)
    out.push_str(",\"flags\":[");
    for (i, f) in world.flags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        put_str(&mut out, &f.name);
        out.push_str(",\"room\":");
        put_str(&mut out, &f.room);
        let _ = write!(
            out,
            ",\"x\":{},\"y\":{},\"color\":{},\"secondaryColor\":{}}}",
            f.x, f.y, f.color, f.secondary_color
        );
    }
    out.push_str("]}");
    out
}

fn write_store(out: &mut String, store: &Store) {
    // 官方 Store 只包含非零资源; 缺失键由 shim 的 Proxy 按 RESOURCES_ALL 返回 0
    out.push('{');
    let mut first = true;
    for (k, v) in store {
        if *v > 0 {
            if !first {
                out.push(',');
            }
            first = false;
            put_str(out, k);
            out.push(':');
            let _ = write!(out, "{}", v);
        }
    }
    out.push('}');
}

fn write_object(out: &mut String, obj: &Obj, world: &World) {
    out.push_str("{\"id\":\"");
    let _ = write!(out, "{:024x}", obj.id);
    out.push_str("\",\"type\":");
    put_str(out, &obj.obj_type);
    out.push_str(",\"x\":");
    let _ = write!(out, "{}", obj.x);
    out.push_str(",\"y\":");
    let _ = write!(out, "{}", obj.y);

    match obj.obj_type.as_str() {
        "creep" => {
            out.push_str(",\"name\":");
            put_str(out, obj.name.as_deref().unwrap_or_default());
            out.push_str(",\"my\":");
            let _ = write!(out, "{}", obj.mine);
            out.push_str(",\"body\":[");
            for (i, p) in obj.body.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str("{\"type\":");
                put_str(out, &p.part_type);
                out.push_str(",\"hits\":");
                let _ = write!(out, "{}", p.hits);
                out.push('}');
            }
            out.push_str("],\"store\":");
            write_store(out, &obj.store);
            out.push_str(",\"storeCapacity\":");
            let _ = write!(out, "{}", obj.store_capacity());
            out.push_str(",\"fatigue\":");
            let _ = write!(out, "{}", obj.fatigue);
            if obj.spawning {
                out.push_str(",\"spawning\":true");
            } else {
                out.push_str(",\"ticksToLive\":");
                match obj.age_time {
                    Some(t) => {
                        let _ = write!(out, "{}", t.saturating_sub(world.tick));
                    }
                    None => out.push_str("null"),
                }
            }
            out.push_str(",\"hits\":");
            let _ = write!(out, "{}", obj.hits);
            out.push_str(",\"hitsMax\":");
            let _ = write!(out, "{}", obj.hits_max);
        }
        "spawn" => {
            write_structure_common(out, obj);
            out.push_str(",\"name\":");
            put_str(out, obj.name.as_deref().unwrap_or_default());
            out.push_str(",\"energy\":");
            let _ = write!(out, "{}", obj.store.get("energy").copied().unwrap_or(0));
            out.push_str(",\"energyCapacity\":");
            let _ = write!(out, "{}", obj.energy_capacity);
            out.push_str(",\"spawning\":");
            match &obj.spawn_job {
                Some(job) => {
                    out.push_str("{\"name\":");
                    put_str(out, &job.name);
                    out.push_str(",\"needTime\":");
                    let _ = write!(out, "{}", job.need_time);
                    out.push_str(",\"remainingTime\":");
                    let _ = write!(out, "{}", job.spawn_time.saturating_sub(world.tick));
                    out.push('}');
                }
                None => out.push_str("null"),
            }
        }
        "extension" | "storage" | "container" | "tower" | "terminal" | "link" | "lab"
        | "powerSpawn" | "factory" | "nuker" => {
            write_structure_common(out, obj);
            if obj.obj_type == "extension" || obj.obj_type == "tower" || obj.obj_type == "link" {
                out.push_str(",\"energy\":");
                let _ = write!(out, "{}", obj.store.get("energy").copied().unwrap_or(0));
                out.push_str(",\"energyCapacity\":");
                let _ = write!(out, "{}", obj.energy_capacity);
            }
            if obj.obj_type == "link" {
                out.push_str(",\"cooldown\":");
                let _ = write!(out, "{}", obj.cooldown);
            }
            if obj.obj_type == "container" || obj.obj_type == "storage" {
                if let Some(t) = obj.next_decay_time {
                    out.push_str(",\"ticksToDecay\":");
                    let _ = write!(out, "{}", t.saturating_sub(world.tick));
                }
            }
        }
        "road" | "constructedWall" | "rampart" => {
            out.push_str(",\"structureType\":");
            put_str(out, &obj.obj_type);
            out.push_str(",\"hits\":");
            let _ = write!(out, "{}", obj.hits);
            out.push_str(",\"hitsMax\":");
            let _ = write!(out, "{}", obj.hits_max);
            if obj.obj_type == "rampart" {
                // rampart 是 OwnedStructure: 归属来自引擎 (此前 shim 一律当作自己的)
                out.push_str(",\"my\":");
                let _ = write!(out, "{}", obj.mine);
                out.push_str(",\"isPublic\":false");
                if let Some(t) = obj.next_decay_time {
                    out.push_str(",\"ticksToDecay\":");
                    let _ = write!(out, "{}", t.saturating_sub(world.tick));
                }
            }
        }
        "source" => {
            out.push_str(",\"energy\":");
            let _ = write!(out, "{}", obj.store.get("energy").copied().unwrap_or(0));
            out.push_str(",\"energyCapacity\":");
            let _ = write!(out, "{}", obj.energy_capacity);
            out.push_str(",\"ticksToRegeneration\":");
            match obj.next_regeneration_time {
                Some(t) => {
                    let _ = write!(out, "{}", t.saturating_sub(world.tick));
                }
                None => out.push_str("null"),
            }
        }
        "mineral" => {
            out.push_str(",\"mineralType\":");
            put_str(out, obj.mineral_type.as_deref().unwrap_or("H"));
            out.push_str(",\"mineralAmount\":");
            let _ = write!(out, "{}", obj.mineral_amount.unwrap_or(0));
        }
        "controller" => {
            out.push_str(",\"my\":");
            let _ = write!(out, "{}", obj.mine);
            out.push_str(",\"level\":");
            let _ = write!(out, "{}", obj.level);
            out.push_str(",\"progress\":");
            let _ = write!(out, "{}", obj.progress);
            let total = constants::CONTROLLER_LEVELS
                .iter()
                .find(|(l, _)| *l == obj.level)
                .map(|(_, p)| *p)
                .unwrap_or(0);
            out.push_str(",\"progressTotal\":");
            let _ = write!(out, "{}", total);
            out.push_str(",\"ticksToDowngrade\":");
            match obj.downgrade_time {
                Some(t) => {
                    let _ = write!(out, "{}", t.saturating_sub(world.tick));
                }
                None => out.push_str("null"),
            }
            out.push_str(",\"safeModeAvailable\":0,\"safeMode\":null,\"upgradeBlocked\":");
            let _ = write!(out, "{}", obj.upgrade_blocked.saturating_sub(world.tick));
            out.push_str(",\"reservation\":");
            match obj.reservation_end_time {
                Some(end) => {
                    out.push_str("{\"username\":\"Nexus\",\"ticksToEnd\":");
                    let _ = write!(out, "{}", end.saturating_sub(world.tick));
                    out.push('}');
                }
                None => out.push_str("null"),
            }
            out.push_str(",\"sign\":");
            match &obj.sign {
                Some(text) => {
                    out.push_str("{\"username\":\"Nexus\",\"text\":");
                    put_str(out, text);
                    out.push_str(",\"time\":0,\"datetime\":0}");
                }
                None => out.push_str("null"),
            }
        }
        "constructionSite" => {
            out.push_str(",\"structureType\":");
            put_str(out, obj.structure_type.as_deref().unwrap_or_default());
            out.push_str(",\"progress\":");
            let _ = write!(out, "{}", obj.progress);
            out.push_str(",\"progressTotal\":");
            let _ = write!(out, "{}", obj.progress_total);
            out.push_str(",\"my\":");
            let _ = write!(out, "{}", obj.mine);
            if let Some(n) = &obj.name {
                out.push_str(",\"name\":");
                put_str(out, n);
            }
        }
        "energy" => {
            out.push_str(",\"energy\":");
            let _ = write!(out, "{}", obj.store.get("energy").copied().unwrap_or(0));
            out.push_str(",\"resourceType\":\"energy\"");
        }
        "tombstone" => {
            out.push_str(",\"store\":");
            write_store(out, &obj.store);
            if let Some(t) = obj.next_decay_time {
                out.push_str(",\"ticksToDecay\":");
                let _ = write!(out, "{}", t.saturating_sub(world.tick));
            }
            out.push_str(",\"creepName\":");
            match &obj.creep_name {
                Some(n) => put_str(out, n),
                None => out.push_str("null"),
            }
        }
        _ => {}
    }
    out.push('}');
}

/// 结构通用字段 (structure_common 的直接写出版本)
fn write_structure_common(out: &mut String, obj: &Obj) {
    out.push_str(",\"structureType\":");
    put_str(out, &obj.obj_type);
    out.push_str(",\"hits\":");
    let _ = write!(out, "{}", obj.hits);
    out.push_str(",\"hitsMax\":");
    let _ = write!(out, "{}", obj.hits_max);
    out.push_str(",\"my\":");
    let _ = write!(out, "{}", obj.mine);
    out.push_str(",\"store\":");
    write_store(out, &obj.store);
    // 超出 RCL 上限被停用的 spawn/extension (官方 isActive() 为 false)
    out.push_str(",\"off\":");
    let _ = write!(out, "{}", obj.off);
    out.push_str(",\"storeCapacity\":");
    let cap = if obj.obj_type == "extension" || obj.obj_type == "spawn" {
        obj.energy_capacity
    } else {
        obj.store_capacity()
    };
    let _ = write!(out, "{}", cap);
}

// ── Value 版 (对拍基准) ───────────────────────────────────

pub fn build_snapshot_value(world: &World, opts: &SnapshotOptions) -> Value {
    let mut terrain = Map::new();
    let mut room_names = Vec::new();
    for (name, room) in &world.rooms {
        room_names.push(name.clone());
        if opts.include_terrain {
            terrain.insert(name.clone(), Value::String(room.terrain_json.clone()));
        }
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

    let mut sites = Vec::new();
    for (name, room) in &world.rooms {
        if opts.always_visible || room.visible() {
            continue;
        }
        for obj in room.objects.values() {
            if obj.obj_type == "constructionSite" && obj.mine {
                let mut v = object_json(obj, world);
                v.as_object_mut().unwrap().insert("room".into(), json!(name));
                sites.push(v);
            }
        }
    }
    let flags: Vec<Value> = world
        .flags
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "room": f.room,
                "x": f.x,
                "y": f.y,
                "color": f.color,
                "secondaryColor": f.secondary_color,
            })
        })
        .collect();

    let (gcl_progress, gcl_total) = world.gcl_level_progress();
    json!({
        "constructionSites": sites,
        "flags": flags,
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
                extra.insert("my".into(), json!(obj.mine));
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
                        "ticksToEnd": end.saturating_sub(world.tick),
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
            if let Some(n) = &obj.name {
                extra.insert("name".into(), json!(n));
            }
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
