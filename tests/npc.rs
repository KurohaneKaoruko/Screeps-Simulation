//! NPC 机制回归测试: invader 侵袭 / source keeper / power bank。
//! 语义来源: docs.screeps.com/power.html (Power Bank 性质) + wiki.screepspl.us/Source_Keeper。
//! 纯引擎驱动 (不经过 V8), 完全确定性。

use screeps_sim::engine::constants;
use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::objects::Obj;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;

fn hex(id: u64) -> String {
    format!("{id:024x}")
}

fn setup(json: &str) -> (World, Movement) {
    let scenario = Scenario::from_json(json).unwrap();
    (build_world(&scenario).unwrap(), Movement::default())
}

fn empty_intents() -> IntentList {
    IntentList::parse("[]")
}

/// 跑 n 个无意图 tick
fn run_quiet(world: &mut World, movement: &mut Movement, n: usize) {
    for _ in 0..n {
        tick::run_tick(world, movement, &empty_intents());
    }
}

fn cheb(ax: u8, ay: u8, bx: u8, by: u8) -> i32 {
    (ax as i32 - bx as i32).abs().max((ay as i32 - by as i32).abs())
}

/// 按 npc_type 过滤 (invader / keeper 都是 obj_type=creep)
fn npcs_of<'a>(world: &'a World, room: &str, npc: &str) -> Vec<&'a Obj> {
    world.rooms[room]
        .objects
        .values()
        .filter(|o| o.npc_type.as_deref() == Some(npc))
        .collect()
}

fn creep_named<'a>(world: &'a World, room: &str, name: &str) -> Option<&'a Obj> {
    world.rooms[room]
        .objects
        .values()
        .find(|o| o.obj_type == "creep" && o.name.as_deref() == Some(name))
}

/// 按类型 + 坐标定位对象 (用于场景里显式放置的结构)
fn obj_at<'a>(world: &'a World, room: &str, obj_type: &str, x: u8, y: u8) -> &'a Obj {
    world.rooms[room]
        .objects
        .values()
        .find(|o| o.obj_type == obj_type && o.x == x && o.y == y)
        .unwrap_or_else(|| panic!("{room} 的 {obj_type} @ ({x},{y}) 不存在"))
}

fn obj_at_id<'a>(world: &'a World, room: &str, id: u64) -> &'a Obj {
    world.rooms[room]
        .objects
        .get(&id)
        .unwrap_or_else(|| panic!("{room} 的对象 {id} 不存在"))
}

fn obj_mut<'a>(world: &'a mut World, room: &str, id: u64) -> &'a mut Obj {
    world
        .rooms
        .get_mut(room)
        .unwrap()
        .objects
        .get_mut(&id)
        .unwrap()
}

/// 直接改坐标 (测试用瞬移; 不经过移动管线)
fn teleport(world: &mut World, room: &str, id: u64, x: u8, y: u8) {
    let o = obj_mut(world, room, id);
    o.x = x;
    o.y = y;
}

fn set_next_spawn(world: &mut World, room: &str, lair_id: u64, t: u64) {
    obj_mut(world, room, lair_id).next_spawn_time = Some(t);
}

// ── 场景 ──

/// invader 侵袭: first=0 / every=8 / lifetime=20 / squad=2。
/// 侵袭者从房间边缘 (25,1) / (48,25) 进入; defender 放在 (25,4) —— 距 TOP 进入点 3 格,
/// 侵袭者立刻用 rangedAttack (近战会触发 defender 的近战反击, 侵袭者会被反杀)。
const INVADER_SCENE: &str = r#"{
    "name": "inv",
    "seed": 1,
    "npcs": { "invaders": { "every": 8, "first": 0, "lifetime": 20, "squad": 2 } },
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "sources": { "positions": [[18, 22]] },
          "controller": { "level": 1, "pos": [28, 27] },
          "structures": [{ "type": "spawn", "pos": [28, 28], "energy": 300 }],
          "creeps": [{ "name": "def",
            "body": ["move", "move", "move",
                     "attack", "attack", "attack", "attack", "attack", "attack", "attack",
                     "attack", "attack", "attack", "attack", "attack", "attack", "attack",
                     "attack", "attack", "attack", "attack"],
            "pos": [25, 4] }] }
    ]
}"#;

/// 双 lair: 两个 lair 各守卫最近的 source (18,22) / (18,30)
const KEEPER_SCENE: &str = r#"{
    "name": "keeper",
    "seed": 1,
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "sources": { "positions": [[18, 22], [18, 30]] },
          "structures": [
            { "type": "keeperLair", "pos": [20, 22] },
            { "type": "keeperLair", "pos": [20, 30] }
          ] }
    ]
}"#;

/// 单 lair + 远处侦察兵 (仇恨圈外)
const KEEPER_GUARD_SCENE: &str = r#"{
    "name": "keeper-guard",
    "seed": 1,
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "sources": { "positions": [[18, 22]] },
          "controller": { "level": 1, "pos": [28, 27] },
          "structures": [{ "type": "keeperLair", "pos": [20, 22] }],
          "creeps": [{ "name": "scout", "body": ["move", "move", "carry", "carry", "work", "work"], "pos": [40, 40] }] }
    ]
}"#;

/// 单 lair + 相邻攻击手 (打 lair 用)
const LAIR_DAMAGE_SCENE: &str = r#"{
    "name": "lair-dmg",
    "seed": 1,
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "sources": { "positions": [[18, 22]] },
          "structures": [{ "type": "keeperLair", "pos": [20, 22] }],
          "creeps": [{ "name": "atk", "body": ["move", "move", "attack", "attack"], "pos": [20, 21] }] }
    ]
}"#;

// ── invader ──

#[test]
fn invader_wave_spawns_attacks_leaves_and_returns() {
    let (mut world, mut movement) = setup(INVADER_SCENE);
    assert!(world.npc_enabled, "配置了 npcs.invaders 应启用 NPC tick 相位");
    assert!(npcs_of(&world, "W5N8", "invader").is_empty(), "初始不应有 invader");

    // tick 0 末尾 (first=0): 第一波
    run_quiet(&mut world, &mut movement, 1);
    let wave = npcs_of(&world, "W5N8", "invader");
    assert_eq!(wave.len(), 2, "第一波应生成 squad=2 个 invader");
    for inv in &wave {
        assert!(!inv.mine && inv.enemy, "invader 是敌方 creep (mine=false, enemy=true)");
        assert_eq!(inv.npc_despawn_time, Some(20), "lifetime=20 → tick 20 撤离");
        // 5 MOVE + 4 ATTACK + 1 RANGED_ATTACK = 10 部件 × 100 hits
        assert_eq!(inv.hits, 1000, "invader 身体 10 部件");
        assert_eq!(inv.body.len(), 10);
        // 官方: 侵袭者从房间边界进入 (内缩 1 格, 不会停在出口格)
        assert!(
            inv.x == 1 || inv.x == 48 || inv.y == 1 || inv.y == 48,
            "invader 应从房间边缘进入 (实际 {:?})",
            (inv.x, inv.y)
        );
    }
    assert!(world.rooms["W5N8"].visible(), "敌方 creep 使房间对玩家可见");
    let wave1_ids: Vec<u64> = wave.iter().map(|o| o.id).collect();

    // 跑到 tick 3 末尾: TOP 进入点 (25,1) 距 defender (25,4) 3 格 → 每 tick 远程 10
    run_quiet(&mut world, &mut movement, 3);
    let def = creep_named(&world, "W5N8", "def").expect("defender 不应在 tick 3 前阵亡");
    assert!(
        def.hits < def.hits_max,
        "invader 应主动攻击最近的玩家 creep (hits={}/{})",
        def.hits,
        def.hits_max
    );

    // 跑到 tick 8 末尾: every=8 → 第二波叠加 (第一波尚未撤离, lifetime=20 > every=8)
    run_quiet(&mut world, &mut movement, 5);
    let at8 = npcs_of(&world, "W5N8", "invader");
    assert_eq!(at8.len(), 4, "第二波到达时第一波仍在");
    assert!(
        at8.iter().filter(|i| i.npc_despawn_time == Some(20)).count() == 2
            && at8.iter().filter(|i| i.npc_despawn_time == Some(28)).count() == 2,
        "第一波 despawn=20, 第二波 despawn=28"
    );

    // 跑到 tick 20 末尾: 第一波到期撤离 (第二/三波在场 → 总数仍为 4)
    run_quiet(&mut world, &mut movement, 12);
    let at20 = npcs_of(&world, "W5N8", "invader");
    assert!(
        at20.iter().all(|i| !wave1_ids.contains(&i.id)),
        "第一波应在 tick 20 全部撤离"
    );
    assert_eq!(
        at20.iter().filter(|i| i.npc_despawn_time == Some(28)).count(),
        2,
        "第二波仍在场"
    );
    assert_eq!(
        at20.iter().filter(|i| i.npc_despawn_time == Some(36)).count(),
        2,
        "tick 16 的第三波已进入"
    );

    // 跑到 tick 28 末尾: 第二波也到期撤离 (第四波 (tick 24) 已在场)
    run_quiet(&mut world, &mut movement, 8);
    let at28 = npcs_of(&world, "W5N8", "invader");
    assert!(
        at28.iter().all(|i| i.npc_despawn_time != Some(28)),
        "第二波应在 tick 28 全部撤离"
    );
    assert_eq!(at28.len(), 4, "稳态下始终有 2 波在场 (lifetime/every ≈ 2.5)");
    assert!(
        world.rooms["W5N8"].objects.values().all(|o| o.obj_type != "tombstone"),
        "invader 撤离不留墓碑 (官方只是离开地图)"
    );
    assert_eq!(world.creeps_lost, 0, "NPC 存亡不计入玩家 creeps_lost");
}

/// 单波侵袭 (every 远大于 lifetime): 到期后房间内不残留任何 invader
#[test]
fn invader_single_wave_leaves_no_trace() {
    let scene = r#"{
        "name": "inv-single",
        "seed": 1,
        "npcs": { "invaders": { "every": 100000, "first": 0, "lifetime": 10, "squad": 2 } },
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "sources": { "positions": [[18, 22]] },
              "controller": { "level": 1, "pos": [28, 27] },
              "structures": [{ "type": "spawn", "pos": [28, 28], "energy": 300 }] }
        ]
    }"#;
    let (mut world, mut movement) = setup(scene);
    run_quiet(&mut world, &mut movement, 1);
    assert_eq!(npcs_of(&world, "W5N8", "invader").len(), 2);

    run_quiet(&mut world, &mut movement, 10); // tick 10 末尾到期
    assert!(
        npcs_of(&world, "W5N8", "invader").is_empty(),
        "到达 lifetime 后 invader 应撤离消失"
    );
    assert!(
        world.rooms["W5N8"].objects.values().all(|o| o.obj_type != "tombstone"),
        "invader 撤离不留墓碑"
    );
    assert_eq!(world.creeps_lost, 0);
}

/// invader AI: 目标 = 最近的玩家对象; 移动用官方方向常量 (斜向优先)
#[test]
fn invader_intents_use_official_directions() {
    let (mut world, mut movement) = setup(INVADER_SCENE);
    run_quiet(&mut world, &mut movement, 1);
    let inv_ids: Vec<u64> = npcs_of(&world, "W5N8", "invader").iter().map(|o| o.id).collect();
    assert_eq!(inv_ids.len(), 2);

    // 把 invader 挪到 (20,25): 最近的玩家对象是 spawn (28,28) (cheb 8 > 3) → move;
    // 方向 (dx=8, dy=3) → (1,1) → 官方 BOTTOM_RIGHT = 4
    for id in &inv_ids {
        teleport(&mut world, "W5N8", *id, 20, 25);
    }
    let intents = screeps_sim::engine::npc::npc_intents(&world);
    assert_eq!(intents.entries.len(), 2, "每个 invader 一条意图");
    for (oid, name, data) in &intents.entries {
        assert!(
            inv_ids.iter().any(|i| hex(*i) == *oid),
            "意图来源应为 invader 之一: {oid}"
        );
        assert_eq!(name, "move", "距离 > 3 时应移动接近而非攻击");
        assert_eq!(data["direction"], 4, "BOTTOM_RIGHT (官方方向常量 4)");
    }
}

// ── source keeper ──

#[test]
fn keeper_lairs_spawn_scoped_keepers_and_rotate_independently() {
    let (mut world, mut movement) = setup(KEEPER_SCENE);
    assert!(world.npc_enabled);

    let lair_a = obj_at(&world, "W5N8", "keeperLair", 20, 22).id;
    let lair_b = obj_at(&world, "W5N8", "keeperLair", 20, 30).id;
    let src_a = obj_at(&world, "W5N8", "source", 18, 22).id;
    let src_b = obj_at(&world, "W5N8", "source", 18, 30).id;

    // prime: 各绑最近 source, 首次生成排在 +300
    assert_eq!(obj_at(&world, "W5N8", "keeperLair", 20, 22).guard_source, Some(src_a));
    assert_eq!(obj_at(&world, "W5N8", "keeperLair", 20, 30).guard_source, Some(src_b));
    assert_eq!(obj_at(&world, "W5N8", "keeperLair", 20, 22).next_spawn_time, Some(300));

    // 两个 lair 都排到 tick 2 到期
    set_next_spawn(&mut world, "W5N8", lair_a, 2);
    set_next_spawn(&mut world, "W5N8", lair_b, 2);
    run_quiet(&mut world, &mut movement, 3); // tick 2 末尾生成

    let keepers = npcs_of(&world, "W5N8", "keeper");
    assert_eq!(keepers.len(), 4, "两个 lair 各生成 2 个 keeper");
    let ka: Vec<&Obj> = keepers.iter().copied().filter(|k| k.guard_lair == Some(lair_a)).collect();
    let kb: Vec<&Obj> = keepers.iter().copied().filter(|k| k.guard_lair == Some(lair_b)).collect();
    assert_eq!((ka.len(), kb.len()), (2, 2), "keeper 按 lair 归属正确分组");
    for k in &ka {
        assert_eq!(k.guard_source, Some(src_a), "A 组 keeper 守卫 A 的 source");
        assert!(!k.mine && k.enemy);
        assert_eq!(k.hits, 4000, "keeper 身体 15+20+5=40 部件");
        assert_eq!(k.npc_type.as_deref(), Some("keeper"));
    }
    for k in &kb {
        assert_eq!(k.guard_source, Some(src_b));
    }
    let old_a: Vec<u64> = ka.iter().map(|k| k.id).collect();
    let old_b: Vec<u64> = kb.iter().map(|k| k.id).collect();

    // lair A 再次到期: 只换 A 的 keeper, B 的 keeper 原样保留
    set_next_spawn(&mut world, "W5N8", lair_a, 6);
    run_quiet(&mut world, &mut movement, 4); // tick 6 末尾

    let keepers2 = npcs_of(&world, "W5N8", "keeper");
    assert_eq!(keepers2.len(), 4, "更替后总数不变 (旧的自杀, 新的补位)");
    let ka2: Vec<&Obj> = keepers2.iter().copied().filter(|k| k.guard_lair == Some(lair_a)).collect();
    let kb2: Vec<u64> = keepers2
        .iter()
        .filter(|k| k.guard_lair == Some(lair_b))
        .map(|k| k.id)
        .collect();
    assert_eq!(ka2.len(), 2);
    assert_eq!(kb2.len(), 2);
    assert_eq!(kb2, old_b, "未到期的 lair B 的 keeper 不应被替换");
    assert!(
        ka2.iter().all(|k| !old_a.contains(&k.id)),
        "lair A 的 keeper 应全部换成新生成的一批"
    );
    // lair 的下次生成排到 now + 300
    assert_eq!(
        obj_at(&world, "W5N8", "keeperLair", 20, 22).next_spawn_time,
        Some(6 + constants::KEEPER_SPAWN_INTERVAL)
    );
}

#[test]
fn keeper_returns_to_source_guard_circle_and_aggros_only_in_range() {
    let (mut world, mut movement) = setup(KEEPER_GUARD_SCENE);
    let lair = obj_at(&world, "W5N8", "keeperLair", 20, 22).id;
    set_next_spawn(&mut world, "W5N8", lair, 1);
    run_quiet(&mut world, &mut movement, 2); // tick 1 末尾生成

    let keepers = npcs_of(&world, "W5N8", "keeper");
    assert_eq!(keepers.len(), 2);
    let kid = keepers[0].id;
    // 出生点应紧贴 lair
    assert!(cheb(keepers[0].x, keepers[0].y, 20, 22) <= 1, "keeper 出生在 lair 邻格");

    // 1) 扔到远处 (避开 scout 所在的 40,40) → 回防守卫圈 (距被守卫 source ≤ KEEPER_GUARD_RANGE)
    teleport(&mut world, "W5N8", kid, 10, 40);
    run_quiet(&mut world, &mut movement, 1);
    let k = obj_at_id(&world, "W5N8", kid);
    assert!(cheb(k.x, k.y, 10, 40) > 0, "keeper 应向 source 方向移动");
    run_quiet(&mut world, &mut movement, 30);
    let k = obj_at_id(&world, "W5N8", kid);
    assert!(
        cheb(k.x, k.y, 18, 22) <= constants::KEEPER_GUARD_RANGE as i32,
        "keeper 应停在守卫圈内 (实际 {:?})",
        (k.x, k.y)
    );

    // 2) 不追击: scout 在 (40,40) 远在仇恨圈外 → 不掉血, keeper 也不接近
    let scout = creep_named(&world, "W5N8", "scout").unwrap();
    let scout_id = scout.id;
    let dist_before = cheb(k.x, k.y, 40, 40);
    run_quiet(&mut world, &mut movement, 3);
    let scout = obj_at_id(&world, "W5N8", scout_id);
    assert_eq!(scout.hits, 600, "仇恨圈外的玩家不应被 keeper 攻击");
    let k = obj_at_id(&world, "W5N8", kid);
    assert!(
        cheb(k.x, k.y, 40, 40) >= dist_before,
        "keeper 不追击: 与圈外玩家的距离不应缩小"
    );

    // 3) 进入仇恨圈 (cheb ≤ 4) → rangedAttack 命中 (5 部件 × 10)
    let (kx, ky) = {
        let k = obj_at_id(&world, "W5N8", kid);
        (k.x, k.y)
    };
    teleport(&mut world, "W5N8", scout_id, kx + 2, ky);
    run_quiet(&mut world, &mut movement, 1);
    let scout = obj_at_id(&world, "W5N8", scout_id);
    assert!(
        scout.hits <= 600 - 50,
        "圈内玩家应被 keeper rangedAttack 命中 (hits={})",
        scout.hits
    );

    // 4) 贴脸 (cheb 1) → 近战 attack 600 (20 部件 × 30); scout 剩余血量会被一击打穿
    let (kx, ky) = {
        let k = obj_at_id(&world, "W5N8", kid);
        (k.x, k.y)
    };
    teleport(&mut world, "W5N8", scout_id, kx + 1, ky);
    run_quiet(&mut world, &mut movement, 1);
    assert!(
        creep_named(&world, "W5N8", "scout").is_none(),
        "贴脸的玩家应被 keeper 近战打死 (20 ATTACK × 30 = 600)"
    );
}

#[test]
fn keeper_lair_damage_resets_spawn_countdown() {
    let (mut world, mut movement) = setup(LAIR_DAMAGE_SCENE);
    let lair = obj_at(&world, "W5N8", "keeperLair", 20, 22).id;
    set_next_spawn(&mut world, "W5N8", lair, 5);
    run_quiet(&mut world, &mut movement, 4); // ticks 0..3, 未到期

    // tick 4: atk (2 ATTACK 部件 = 60 伤害) 打 lair
    let atk = creep_named(&world, "W5N8", "atk").unwrap().id;
    let intents = IntentList::parse(&format!(
        r#"[{{"o":"{}","n":"attack","d":{{"id":"{}"}}}}]"#,
        hex(atk),
        hex(lair)
    ));
    tick::run_tick(&mut world, &mut movement, &intents);

    let l = obj_at(&world, "W5N8", "keeperLair", 20, 22);
    assert_eq!(l.hits, 50_000 - 60, "lair 应受到 60 点伤害");
    assert_eq!(
        l.next_spawn_time,
        Some(4 + constants::KEEPER_RESPAWN_AFTER_DAMAGE),
        "lair 受扰后生成倒计时重置为 now+300"
    );
    assert!(
        npcs_of(&world, "W5N8", "keeper").is_empty(),
        "倒计时重置后不应在本 tick 生成 keeper"
    );

    // 原定的 tick 5 已作废: 再跑 5 tick 仍无 keeper
    run_quiet(&mut world, &mut movement, 5);
    assert!(
        npcs_of(&world, "W5N8", "keeper").is_empty(),
        "受扰后的倒计时期间 (300t) 不生成 keeper"
    );
}

// ── power bank ──

#[test]
fn power_bank_decays_linearly_and_expires() {
    let scene = r#"{
        "name": "bank-decay",
        "seed": 1,
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "structures": [{ "type": "powerBank", "pos": [10, 10], "hits": 100000,
                               "store": { "power": 3000 } }] }
        ]
    }"#;
    let (mut world, mut movement) = setup(scene);
    assert!(world.npc_enabled, "powerBank 也应启用 NPC tick 相位");

    let bank = obj_at(&world, "W5N8", "powerBank", 10, 10);
    assert_eq!(bank.hits, 100000);
    assert_eq!(bank.hits_max, constants::POWER_BANK_HITS, "hitsMax 兜底为官方 2M");
    assert_eq!(bank.store.get("power"), Some(&3000));
    assert_eq!(bank.next_decay_time, Some(constants::POWER_BANK_DECAY_STEP));

    // 前 99 tick 不衰减 (下一次衰减排在 tick 100)
    run_quiet(&mut world, &mut movement, 99);
    assert_eq!(obj_at(&world, "W5N8", "powerBank", 10, 10).hits, 100000);

    // tick 100 末尾: 一步衰减 = ceil(2M × 100/5000) = 40000; power 每步 100
    run_quiet(&mut world, &mut movement, 2);
    let bank = obj_at(&world, "W5N8", "powerBank", 10, 10);
    assert_eq!(bank.hits, 60000, "hits 按 40000/步 线性衰减");
    assert_eq!(bank.store.get("power"), Some(&2900), "power 同步线性衰减");
    assert_eq!(bank.next_decay_time, Some(200), "下一衰减排到 +100");
}

#[test]
fn power_bank_decay_to_zero_destroys_with_event() {
    let scene = r#"{
        "name": "bank-expire",
        "seed": 1,
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "structures": [{ "type": "powerBank", "pos": [10, 10], "hits": 40,
                               "store": { "power": 3000 } }] }
        ]
    }"#;
    let (mut world, mut movement) = setup(scene);
    let bank_id = obj_at(&world, "W5N8", "powerBank", 10, 10).id;
    run_quiet(&mut world, &mut movement, 101); // tick 100 末尾衰减一步即归零

    assert!(
        world.rooms["W5N8"].objects.values().all(|o| o.obj_type != "powerBank"),
        "hits 归零的 powerBank 应消失"
    );
    assert!(
        world.rooms["W5N8"]
            .events
            .iter()
            .any(|e| e.event == screeps_sim::engine::intents::EVENT_OBJECT_DESTROYED
                && e.object_id == bank_id),
        "应产生 EVENT_OBJECT_DESTROYED"
    );
}

#[test]
fn power_bank_bounces_half_damage_and_drops_power_when_destroyed() {
    let scene = r#"{
        "name": "bank-fight",
        "seed": 1,
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "structures": [{ "type": "powerBank", "pos": [10, 10], "hits": 100,
                               "store": { "power": 1234 } }],
              "creeps": [{ "name": "dig", "body": ["move", "attack", "attack"], "pos": [10, 11] }] }
        ]
    }"#;
    let (mut world, mut movement) = setup(scene);
    let bank = obj_at(&world, "W5N8", "powerBank", 10, 10).id;
    let dig = creep_named(&world, "W5N8", "dig").unwrap().id;
    let intents = IntentList::parse(&format!(
        r#"[{{"o":"{}","n":"attack","d":{{"id":"{}"}}}}]"#,
        hex(dig),
        hex(bank)
    ));

    // 第 1 击: 60 伤害 → bank 100-60=40, 攻击者被反弹 30
    tick::run_tick(&mut world, &mut movement, &intents);
    let b = obj_at(&world, "W5N8", "powerBank", 10, 10);
    assert_eq!(b.hits, 40);
    let d = obj_at_id(&world, "W5N8", dig);
    assert_eq!(d.hits, 300 - 30, "50% 伤害 (30) 反弹给攻击者");

    // 第 2 击: 摧毁 → 掉落全部 power 于原位
    tick::run_tick(&mut world, &mut movement, &intents);
    assert!(
        world.rooms["W5N8"].objects.values().all(|o| o.obj_type != "powerBank"),
        "powerBank 应被摧毁"
    );
    let drop = world.rooms["W5N8"]
        .objects
        .values()
        .find(|o| o.obj_type == "energy" && o.x == 10 && o.y == 10)
        .expect("摧毁后应在原位掉落 power");
    assert_eq!(drop.resource_type(), "power");
    // 掉落量 = 全部 power 减去掉落资源衰减 (ENERGY_DECAY: 每 tick ceil(amount/1000) = 2)
    let dropped = drop.store.get("power").copied().unwrap_or(0);
    assert!(
        (1232..=1234).contains(&dropped),
        "应掉落全部 power (含 1 tick 掉落衰减, 实际 {dropped})"
    );
    // 摧毁一击同样反弹
    assert_eq!(obj_at_id(&world, "W5N8", dig).hits, 300 - 60);
}

// ── 快照 (JS API 面) ──

#[test]
fn snapshot_exposes_keeper_lair_and_power_bank_fields() {
    use screeps_sim::engine::snapshot::{build_snapshot, CpuView, SnapshotOptions};
    let scene = r#"{
        "name": "npc-snap",
        "seed": 1,
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "sources": { "positions": [[18, 22]] },
              "structures": [
                { "type": "keeperLair", "pos": [20, 22] },
                { "type": "powerBank", "pos": [10, 10], "store": { "power": 3000 } }
              ] }
        ]
    }"#;
    let (mut world, mut movement) = setup(scene);
    run_quiet(&mut world, &mut movement, 3);

    let snap = build_snapshot(
        &world,
        &SnapshotOptions {
            cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
            always_visible: true,
            include_terrain: false,
        },
    );
    let json: serde_json::Value = serde_json::from_str(&snap).unwrap();
    let objects = json["rooms"]["W5N8"]["objects"].as_array().unwrap();

    let lair = objects
        .iter()
        .find(|o| o["type"] == "keeperLair")
        .expect("快照应含 keeperLair");
    assert_eq!(lair["ticksToSpawn"], 297, "ticksToSpawn = next_spawn_time - time");
    assert_eq!(lair["hitsMax"], constants::KEEPER_LAIR_HITS);

    let bank = objects
        .iter()
        .find(|o| o["type"] == "powerBank")
        .expect("快照应含 powerBank");
    assert_eq!(bank["power"], 3000, "powerBank 暴露 power 字段");
    assert_eq!(bank["ticksToDecay"], 97, "ticksToDecay = next_decay_time - time");
}

