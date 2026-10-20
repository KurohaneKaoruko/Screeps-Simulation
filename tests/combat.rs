//! 战斗机制回归测试: 安全模式 (safe mode) + 核弹 (nuke)。
//! 语义来源: docs.screeps.com/api/#StructureController.activateSafeMode 与
//! docs.screeps.com/api/#StructureNuker.launchNuke / power.html:
//! - 安全模式: 消耗一次激活次数, 持续 SAFE_MODE_DURATION (20000); 两次激活间隔
//!   SAFE_MODE_COOLDOWN (50000); 一个 shard 同时只能一个房间处于安全模式; 激活期间非房主
//!   的攻击被拦截 (房主自己的攻击不受影响); 每次控制器升级 +1 次激活。
//! - 核弹: 消耗满仓 energy(300000)+ghodium(5000), 冷却 NUKER_COOLDOWN; 飞行 NUKE_LAND_TIME
//!   (50000) 后落地: 清空全房间 creep/工地/掉落物/墓碑/废墟, 落点 NUKE_DAMAGE[0] (1e7)、
//!   5×5 内 NUKE_DAMAGE[2] (5e6) 结构伤害, 取消该房安全模式并把冷却清零, 控制器
//!   upgradeBlocked 200 tick。
//! 纯引擎驱动 (不经过 V8), 完全确定性。

use screeps_sim::engine::combat::{self, Nuke};
use screeps_sim::engine::intents::{self, IntentList};
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::snapshot::{build_snapshot, build_snapshot_value, CpuView, SnapshotOptions};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;
use serde_json::{json, Value};

fn setup(json_str: &str) -> (World, Movement) {
	let scenario = Scenario::from_json(json_str).unwrap();
	(build_world(&scenario).unwrap(), Movement::default())
}

/// 构造 `[{o: idHex, n: name, d: data}]` 形式的意图 JSON
fn intents_json(entries: &[(u64, &str, Value)]) -> String {
	let items: Vec<Value> = entries
		.iter()
		.map(|(id, n, d)| json!({ "o": format!("{:024x}", id), "n": n, "d": d }))
		.collect();
	Value::Array(items).to_string()
}

fn obj_by_type(world: &World, room: &str, t: &str) -> u64 {
	world.rooms[room]
		.objects
		.values()
		.find(|o| o.obj_type == t)
		.unwrap_or_else(|| panic!("{room} 无 {t}"))
		.id
}

fn creep_named(world: &World, name: &str) -> u64 {
	for room in world.rooms.values() {
		for obj in room.objects.values() {
			if obj.obj_type == "creep" && obj.name.as_deref() == Some(name) {
				return obj.id;
			}
		}
	}
	panic!("creep {name} 不存在");
}

fn hits_of(world: &World, id: u64) -> u32 {
	for room in world.rooms.values() {
		if let Some(o) = room.objects.get(&id) {
			return o.hits;
		}
	}
	panic!("对象 {id:x} 不存在");
}

fn snapshot_opts() -> SnapshotOptions {
	SnapshotOptions {
		cpu: CpuView { limit: 20.0, tick_limit: 500.0, bucket: 10000.0 },
		always_visible: true,
		include_terrain: true,
	}
}

// ── 场景 ──

/// 单房: RCL8 控制器 (可激活安全模式 1 次) + 满仓 nuker
const BASE_SCENE: &str = r#"{
	"name": "combat",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "count": 0 },
		  "controller": { "level": 8, "pos": [25, 25], "safeModeAvailable": 1 },
		  "structures": [
			{ "type": "nuker", "pos": [40, 40], "store": { "energy": 300000, "G": 5000 } }
		  ] }
	]
}"#;

/// 双房: 各自 RCL5 控制器, 均可激活安全模式 1 次
const TWO_ROOM_SCENE: &str = r#"{
	"name": "two-safe",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" }, "sources": { "count": 0 },
		  "controller": { "level": 5, "pos": [25, 25], "safeModeAvailable": 1 } },
		{ "name": "W4N8", "terrain": { "kind": "flat" }, "sources": { "count": 0 },
		  "controller": { "level": 5, "pos": [25, 25], "safeModeAvailable": 1 } }
	]
}"#;

/// 攻防场景: 我方 defender([10,10]) + guard([12,10]), 敌方 raider([11,10]) 相邻
const ATTACK_SCENE: &str = r#"{
	"name": "attack",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "count": 0 },
		  "controller": { "level": 5, "pos": [25, 25], "safeModeAvailable": 1 },
		  "creeps": [
			{ "name": "defender", "body": ["move", "move"], "pos": [10, 10] },
			{ "name": "guard", "body": ["attack", "move"], "pos": [12, 10] },
			{ "name": "raider", "body": ["attack", "move"], "pos": [11, 10], "enemy": true }
		  ] }
	]
}"#;

/// 核弹落点场景: 落点 [10,10] 有 road, [13,13] 有 road (距离 3, 应存活), [10,11] 有 victim
const NUKE_SCENE: &str = r#"{
	"name": "nuke",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "count": 0 },
		  "controller": { "level": 8, "pos": [25, 25], "safeModeAvailable": 1 },
		  "structures": [
			{ "type": "nuker", "pos": [40, 40], "store": { "energy": 300000, "G": 5000 } },
			{ "type": "road", "pos": [10, 10] },
			{ "type": "road", "pos": [13, 13] }
		  ],
		  "creeps": [ { "name": "victim", "body": ["move"], "pos": [10, 11] } ] }
	]
}"#;

// ── 安全模式 ──

/// 激活安全模式: 消耗一次激活次数, 设定持续与冷却到期 tick
#[test]
fn activate_safe_mode_consumes_charge_and_sets_timers() {
	let (mut world, _mv) = setup(BASE_SCENE);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	assert_eq!(world.rooms["W5N8"].objects[&ctrl].safe_mode_available, 1);

	let t0 = world.tick;
	assert!(combat::activate_safe_mode(&mut world, ctrl), "有次数且无冷却 → 激活成功");
	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(c.safe_mode_until, t0 + 20_000, "持续 20000 tick");
	assert_eq!(c.safe_mode_available, 0, "消耗一次激活次数");
	assert_eq!(c.safe_mode_cooldown_until, t0 + 50_000, "冷却 50000 tick");
	assert!(combat::safe_mode_active(&world, "W5N8"));
}

/// 无可用激活次数 → 激活失败, 状态不变
#[test]
fn activate_safe_mode_rejected_without_available_charge() {
	let scene = BASE_SCENE.replace("\"safeModeAvailable\": 1", "\"safeModeAvailable\": 0");
	let (mut world, _mv) = setup(&scene);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	assert!(!combat::activate_safe_mode(&mut world, ctrl), "无次数 → 失败");
	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(c.safe_mode_until, 0);
	assert!(!combat::safe_mode_active(&world, "W5N8"));
}

/// 冷却期内即使安全模式已过期也不能再次激活
#[test]
fn safe_mode_cooldown_blocks_reactivation() {
	let (mut world, _mv) = setup(BASE_SCENE);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	assert!(combat::activate_safe_mode(&mut world, ctrl));
	// 补一次激活次数 (模拟另一次升级), 并手动让安全模式到期
	{
		let c = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&ctrl).unwrap();
		c.safe_mode_available = 1;
		c.safe_mode_until = world.tick; // 到期
	}
	assert!(!combat::safe_mode_active(&world, "W5N8"), "安全模式已过期");
	assert!(!combat::activate_safe_mode(&mut world, ctrl), "冷却中 → 失败");
	assert_eq!(world.rooms["W5N8"].objects[&ctrl].safe_mode_available, 1, "失败不消耗次数");
}

/// 一个 shard 同时只能一个房间处于安全模式 (先到先得)
#[test]
fn only_one_room_in_safe_mode_per_shard() {
	let (mut world, mut mv) = setup(TWO_ROOM_SCENE);
	let c1 = obj_by_type(&world, "W5N8", "controller");
	let c2 = obj_by_type(&world, "W4N8", "controller");
	let json = intents_json(&[(c1, "activateSafeMode", json!({})), (c2, "activateSafeMode", json!({}))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&json));

	let active: Vec<&str> = ["W5N8", "W4N8"]
		.iter()
		.copied()
		.filter(|r| combat::safe_mode_active(&world, r))
		.collect();
	assert_eq!(active.len(), 1, "恰有一个房间进入安全模式 (实际: {active:?})");
	assert!(combat::safe_mode_active(&world, "W5N8"), "低 id 控制器先结算");
	assert_eq!(world.rooms["W4N8"].objects[&c2].safe_mode_available, 1, "失败方不消耗次数");
}

/// 无安全模式时, 敌方近战攻击造成伤害 (基线)
#[test]
fn hostile_attack_damages_creep_without_safe_mode() {
	let (mut world, mut mv) = setup(ATTACK_SCENE);
	let defender = creep_named(&world, "defender");
	let raider = creep_named(&world, "raider");
	let json = intents_json(&[(raider, "attack", json!({ "id": format!("{:024x}", defender) }))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, defender), 200 - 30, "ATTACK_POWER=30 应扣血");
}

/// 安全模式激活期间: 敌方攻击被拦截, 房主自己的攻击不受影响
#[test]
fn safe_mode_blocks_hostile_attack_but_not_owner() {
	let (mut world, mut mv) = setup(ATTACK_SCENE);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	let defender = creep_named(&world, "defender");
	let guard = creep_named(&world, "guard");
	let raider = creep_named(&world, "raider");
	assert!(combat::activate_safe_mode(&mut world, ctrl));

	let json = intents_json(&[
		(raider, "attack", json!({ "id": format!("{:024x}", defender) })),
		(guard, "attack", json!({ "id": format!("{:024x}", raider) })),
	]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&json));

	assert_eq!(hits_of(&world, defender), 200, "敌方攻击被安全模式拦截, defender 满血");
	assert_eq!(hits_of(&world, raider), 170, "房主自己的攻击仍生效 (200-30)");
}

/// 控制器升级授予一次安全模式激活
#[test]
fn controller_upgrade_grants_safe_mode_charge() {
	let scene = r#"{
		"name": "upgrade",
		"seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "sources": { "count": 0 },
			  "controller": { "level": 1, "pos": [25, 25], "progress": 199, "safeModeAvailable": 0 },
			  "creeps": [ { "name": "upgrader", "body": ["work", "move"], "pos": [26, 25], "store": { "energy": 50 } } ] }
		]
	}"#;
	let (mut world, mut mv) = setup(scene);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	let upgrader = creep_named(&world, "upgrader");
	assert_eq!(world.rooms["W5N8"].objects[&ctrl].safe_mode_available, 0);

	let json = intents_json(&[(upgrader, "upgradeController", json!({ "id": format!("{:024x}", ctrl) }))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&json));

	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(c.level, 2, "199 + 1 WORK = 200 → RCL2");
	assert_eq!(c.safe_mode_available, 1, "升级授予一次激活");
}

// ── 核弹 ──

/// 发射核弹: 消耗满仓 energy + ghodium, 设定 nuker 冷却, 登记在飞核弹
#[test]
fn nuke_launch_consumes_energy_and_ghodium() {
	let (mut world, mut mv) = setup(BASE_SCENE);
	let nuker = obj_by_type(&world, "W5N8", "nuker");
	let t0 = world.tick;
	let json = intents_json(&[(nuker, "launchNuke", json!({ "x": 10, "y": 10, "roomName": "W5N8" }))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&json));

	let n = &world.rooms["W5N8"].objects[&nuker];
	assert_eq!(n.store.get("energy").copied().unwrap_or(0), 0, "扣光 energy");
	assert_eq!(n.store.get("G").copied().unwrap_or(0), 0, "扣光 ghodium");
	assert_eq!(n.cooldown_time, t0 + 100_000, "NUKER_COOLDOWN=100000");
	assert_eq!(world.nukes.len(), 1);
	let nuke = &world.nukes[0];
	assert_eq!((nuke.x, nuke.y), (10, 10));
	assert_eq!(nuke.target_room, "W5N8");
	assert_eq!(nuke.land_tick, t0 + 50_000, "NUKE_LAND_TIME=50000");
}

/// 资源不足 / 目标非法 → 不发射
#[test]
fn nuke_launch_rejected_without_resources_or_with_bad_target() {
	let (mut world, mut mv) = setup(BASE_SCENE);
	let nuker = obj_by_type(&world, "W5N8", "nuker");
	// 目标坐标越界 (x=50): 意图阶段即被拒
	let bad = intents_json(&[(nuker, "launchNuke", json!({ "x": 50, "y": 10, "roomName": "W5N8" }))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&bad));
	assert!(world.nukes.is_empty(), "越界坐标不发射");
	assert_eq!(world.rooms["W5N8"].objects[&nuker].store.get("energy").copied().unwrap_or(0), 300_000);

	// 合法目标但 ghodium 不足 → 结算阶段拒绝
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&nuker).unwrap().store.insert("G".into(), 10);
	let ok = intents_json(&[(nuker, "launchNuke", json!({ "x": 10, "y": 10, "roomName": "W5N8" }))]);
	tick::run_tick(&mut world, &mut mv, &IntentList::parse(&ok));
	assert!(world.nukes.is_empty(), "ghodium 不足不发射");
	assert_eq!(world.rooms["W5N8"].objects[&nuker].store.get("energy").copied().unwrap_or(0), 300_000);
}

/// 核弹落地: 清空全房间 creep / 掉落物, 落点及 5×5 内结构受创 (无视 rampart), 远处结构无损
#[test]
fn nuke_impact_clears_room_and_damages_structures() {
	let (mut world, mut mv) = setup(NUKE_SCENE);
	// 预置一堆掉落资源 (落地应被清除)
	intents::create_dropped(&mut world, "W5N8", 20, 20, "energy", 500);
	assert!(world.rooms["W5N8"].objects.values().any(|o| o.obj_type == "energy"));

	let t0 = world.tick;
	world.nukes.push(Nuke {
		id: 0xabc,
		launch_room: "W4N8".into(),
		target_room: "W5N8".into(),
		x: 10,
		y: 10,
		land_tick: t0,
	});
	tick::run_tick(&mut world, &mut mv, &IntentList { entries: vec![] });

	let room = &world.rooms["W5N8"];
	assert!(world.nukes.is_empty(), "落地后从在飞列表移除");
	assert!(!room.objects.values().any(|o| o.obj_type == "creep"), "全房间 creep 被清除");
	assert!(!room.objects.values().any(|o| o.obj_type == "energy"), "掉落物被清除");
	assert!(
		!room.objects.values().any(|o| o.obj_type == "road" && o.x == 10 && o.y == 10),
		"落点结构被摧毁 (1e7 伤害)"
	);
	assert!(
		room.objects.values().any(|o| o.obj_type == "road" && o.x == 13 && o.y == 13),
		"距离 3 的结构存活"
	);
	assert!(room.objects.values().any(|o| o.obj_type == "controller"), "控制器不受伤害");
}

/// 核弹落地: 取消该房安全模式并把冷却清零, 控制器 upgradeBlocked 200 tick
#[test]
fn nuke_impact_cancels_safe_mode_and_blocks_upgrade() {
	let (mut world, mut mv) = setup(NUKE_SCENE);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	assert!(combat::activate_safe_mode(&mut world, ctrl));
	assert!(combat::safe_mode_active(&world, "W5N8"));

	let t0 = world.tick;
	world.nukes.push(Nuke {
		id: 0xdef,
		launch_room: "W4N8".into(),
		target_room: "W5N8".into(),
		x: 10,
		y: 10,
		land_tick: t0,
	});
	tick::run_tick(&mut world, &mut mv, &IntentList { entries: vec![] });

	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(c.safe_mode_until, 0, "安全模式被取消");
	assert_eq!(c.safe_mode_cooldown_until, 0, "冷却被清零");
	assert_eq!(c.upgrade_blocked, t0 + 200, "CONTROLLER_NUKE_BLOCKED_UPGRADE=200");
	assert!(!combat::safe_mode_active(&world, "W5N8"));
}

// ── 快照 ──

/// 快照暴露控制器安全模式字段与在飞核弹, 且快路径与 Value 路径一致
#[test]
fn snapshot_exposes_safe_mode_and_inflight_nukes() {
	let (mut world, mut mv) = setup(NUKE_SCENE);
	let ctrl = obj_by_type(&world, "W5N8", "controller");
	let t0 = world.tick;
	assert!(combat::activate_safe_mode(&mut world, ctrl));
	world.nukes.push(Nuke {
		id: 0x999,
		launch_room: "W4N8".into(),
		target_room: "W5N8".into(),
		x: 10,
		y: 10,
		land_tick: t0 + 50_000,
	});
	// 跑一 tick 让快照携带事件/在飞核弹 (核弹未落地)
	tick::run_tick(&mut world, &mut mv, &IntentList { entries: vec![] });

	let opts = snapshot_opts();
	let fast: Value = serde_json::from_str(&build_snapshot(&world, &opts)).unwrap();
	let value = build_snapshot_value(&world, &opts);
	assert_eq!(fast, value, "快路径与 Value 路径必须一致");

	let room = &fast["rooms"]["W5N8"];
	assert_eq!(room["nukes"][0]["timeToLand"], json!(50_000 - 1), "已过 1 tick");
	assert_eq!(room["nukes"][0]["launchRoomName"], json!("W4N8"));
	assert_eq!(room["nukes"][0]["x"], json!(10));

	let controller = room["objects"]
		.as_array()
		.unwrap()
		.iter()
		.find(|o| o["type"] == json!("controller"))
		.unwrap();
	assert_eq!(controller["safeModeAvailable"], json!(0), "激活后已消耗");
	assert_eq!(controller["safeMode"], json!(20_000 - 1), "剩余 tick (官方语义: 到期 tick - 当前 tick)");
	assert_eq!(controller["safeModeCooldown"], json!(50_000 - 1), "冷却剩余 tick");
}
