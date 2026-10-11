//! JS 侧机制测试: 用微型 bundle (非 bot) 直接驱动 V8 + shim, 验证快照/寻路/意图回收链路。

use nexus_sim::engine::movement::Movement;
use nexus_sim::engine::scenario::{build_world, Scenario};
use nexus_sim::engine::snapshot::{build_snapshot, CpuView, SnapshotOptions};
use nexus_sim::engine::tick;
use nexus_sim::engine::intents::IntentList;
use nexus_sim::runtime::BotRuntime;

const TWO_ROOMS: &str = r#"{
	"name": "t",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "controller": { "level": 1, "pos": [28, 27] },
		  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }] },
		{ "name": "W5N7", "terrain": { "kind": "flat" } }
	]
}"#;

/// deno_core 的 V8 isolate 需要 tokio 上下文
fn in_tokio<F: FnOnce()>(f: F) {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("tokio runtime");
	rt.block_on(async { f() });
}

fn run_once(bundle: &str, world: &mut nexus_sim::engine::world::World) -> (Vec<(String, String, serde_json::Value)>, Vec<String>) {
	let snapshot = build_snapshot(
		world,
		&SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: false,
		},
	);
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let result = bot.run_tick(&serde_json::to_string(&snapshot).unwrap(), "{}").unwrap();
	(IntentList::parse(&result.intents_json).entries, result.console)
}

/// PathFinder 跨房寻路: W5N8 (25,25) → W5N7 (25,25) 路径应跨过出口并到达目标房
#[test]
fn pathfinder_cross_room() {
	in_tokio(|| pathfinder_cross_room_inner());
}

fn pathfinder_cross_room_inner() {
	let scenario = Scenario::from_json(TWO_ROOMS).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const from = new RoomPosition(25, 25, "W5N8");
			const to = new RoomPosition(25, 25, "W5N7");
			const ret = PathFinder.search(from, { pos: to, range: 0 });
			const last = ret.path[ret.path.length - 1];
			console.log(JSON.stringify({
				incomplete: ret.incomplete,
				ops: ret.ops,
				cost: ret.cost,
				steps: ret.path.length,
				lastRoom: last ? last.roomName : null,
				lastXY: last ? [last.x, last.y] : null,
				firstRooms: ret.path.slice(0, 3).map(p => p.roomName + ":" + p.x + "," + p.y),
			}));
		};
	"#;
	let (_, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["incomplete"], false, "跨房路径应完整: {info}");
	assert_eq!(info["lastRoom"], "W5N7", "终点应在目标房: {info}");
	assert_eq!(info["lastXY"][0], 25, "终点 x: {info}");
	let steps = info["steps"].as_u64().unwrap();
	assert!((45..=75).contains(&steps), "跨一房 50 格直线 ≈ 51±步, 实际 {steps}");
}

/// describeExits / findRoute / getRoomStatus: 地图 API 双房间连通
#[test]
fn map_apis_two_rooms() {
	in_tokio(|| map_apis_two_rooms_inner());
}

fn map_apis_two_rooms_inner() {
	let scenario = Scenario::from_json(TWO_ROOMS).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const exits = Game.map.describeExits("W5N8");
			const route = Game.map.findRoute("W5N8", "W5N7");
			const status = Game.map.getRoomStatus("W5N7");
			console.log(JSON.stringify({
				e5: exits["5"],
				routeLen: Array.isArray(route) ? route.length : route,
				routeExit: Array.isArray(route) ? route[0].exit : null,
				routeRoom: Array.isArray(route) ? route[0].room : null,
				status: status.status,
			}));
		};
	"#;
	let (_, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["e5"], "W5N7", "W5N8 南出口应为 W5N7: {info}");
	assert_eq!(info["routeLen"], 1, "路由一跳: {info}");
	assert_eq!(info["routeExit"], 5, "路由方向 BOTTOM: {info}");
	assert_eq!(info["routeRoom"], "W5N7", "路由下一房: {info}");
	assert_eq!(info["status"], "normal", "房间状态: {info}");
}

/// moveTo 端到端: 单位向 source 移动, 意图经引擎结算后位置真实前进
#[test]
fn moveto_end_to_end() {
	in_tokio(|| moveto_end_to_end_inner());
}

fn moveto_end_to_end_inner() {
	let scenario = Scenario::from_json(TWO_ROOMS).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let movement = &mut Movement::default();
	// 注入一只 creep
	use nexus_sim::engine::objects::{creep_obj, BodyPart};
	let id = nexus_sim::engine::world::alloc_id(&mut world.next_id);
	let body: Vec<BodyPart> = ["move", "move", "move"]
		.iter()
		.map(|p| BodyPart { part_type: p.to_string(), hits: 100 })
		.collect();
	let creep = creep_obj(id, "SC1", 25, 25, body);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(id, creep);

	let bundle = r#"
		exports.loop = function() {
			const creep = Object.values(Game.creeps)[0];
			if (!creep) { console.log("no-creep"); return; }
			if (Memory.started === undefined) Memory.started = Game.time;
			const source = creep.room.find(FIND_SOURCES)[0];
			const code = creep.moveTo(source, { reusePath: 5 });
			console.log(JSON.stringify({ code, started: Memory.started, x: creep.pos.x, y: creep.pos.y }));
		};
	"#;
	// 单个 BotRuntime 贯穿整个场景: 覆盖 Memory 往返、跨 tick 全局持久化与 _move 复用缓存
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let mut memory = String::from("{}");
	let mut pos = (25u8, 25u8);
	for _ in 0..12 {
		let snapshot = build_snapshot(
			&world,
			&SnapshotOptions {
				cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
				always_visible: false,
			},
		);
		let result = bot
			.run_tick(&serde_json::to_string(&snapshot).unwrap(), &memory)
			.unwrap();
		memory = result.memory.clone();
		let info: serde_json::Value =
			serde_json::from_str(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info["code"], 0, "moveTo 应返回 OK: {info}");
		// Memory 首 tick 写入后应跨 tick 保留 (此前每 tick 都新建 runtime 且传 "{}", 测不到)
		assert_eq!(info["started"], 0, "Memory 应跨 tick 往返: {info}");

		let intents = IntentList::parse(&result.intents_json);
		tick::run_tick(&mut world, movement, &intents);
		pos = (world.rooms["W5N8"].objects[&id].x, world.rooms["W5N8"].objects[&id].y);
	}
	assert!(memory.contains("\"started\""), "Memory 应被序列化回传: {memory}");
	// 12 tick × 3 MOVE 无负重 → 直线距离 6 格内必到达 source (18,22) 邻域
	let dist = (pos.0 as i32 - 18).abs().max((pos.1 as i32 - 22).abs());
	assert!(dist <= 1, "12 tick 后应到达 source 邻域, 实际 {pos:?} 距离 {dist}");
}

// ═════════════════════════ 战斗 API (JS shim 层) ═════════════════════════

/// 战斗场景: 敌我两只 creep 相邻 + 中立控制器 + 墙
const COMBAT_SCENE: &str = r#"{
	"name": "combat",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "controller": { "level": 0, "pos": [28, 27], "mine": false },
		  "structures": [
		  	{ "type": "spawn", "pos": [25, 25], "energy": 300 },
		  	{ "type": "constructedWall", "pos": [22, 20] }
		  ],
		  "creeps": [
		  	{ "name": "Ally", "body": ["attack", "ranged_attack", "heal", "work", "claim", "carry", "move"], "pos": [20, 20] },
		  	{ "name": "Foe", "body": ["attack", "move"], "pos": [21, 20], "enemy": true }
		  ]
		}
	]
}"#;

/// 战斗 API 返回码与意图提交: attack/rangedAttack/heal/dismantle/claim/reserve/sign/attackController
#[test]
fn combat_apis_submit_intents() {
	in_tokio(|| combat_apis_submit_intents_inner());
}

fn combat_apis_submit_intents_inner() {
	let scenario = Scenario::from_json(COMBAT_SCENE).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const ally = Game.creeps.Ally;
			// Game.creeps 只含我方 (官方语义), 敌人经 FIND_HOSTILE_CREEPS
			const foe = ally.room.find(FIND_HOSTILE_CREEPS)[0];
			const wall = ally.room.find(FIND_STRUCTURES).find(s => s.structureType === "constructedWall");
			const ctrl = ally.room.controller;
			const out = { hasFoe: !!foe, foeMy: foe ? foe.my : null };
			out.attack = ally.attack(foe);
			out.rangedAttack = ally.rangedAttack(foe);
			out.rangedMassAttack = ally.rangedMassAttack();
			out.healSelf = ally.heal(ally);
			out.rangedHealSelf = ally.rangedHeal(ally);
			out.dismantleWall = ally.dismantle(wall);
			out.sign = ally.signController(ctrl, "hi");
			// 控制器无 owner/reservation → attackController ERR_INVALID_TARGET (先于距离检查)
			out.attackControllerNeutral = ally.attackController(ctrl);
			// 距离控制器 8 格: claim/reserve 应 ERR_NOT_IN_RANGE
			out.ctrlRange = ally.pos.getRangeTo(ctrl.pos.x, ctrl.pos.y);
			out.claimFar = ally.claimController(ctrl);
			out.reserveFar = ally.reserveController(ctrl);
			// 攻击友军自身 (my creep) 官方不禁止: attack 自己应 ERR_INVALID_TARGET (id 相同)
			out.attackSelf = ally.attack(ally);
			// 无 attack 部件路径: 用 heal 单位攻击 → 但 ally 有 attack, 改测无部件的 Foe
			console.log(JSON.stringify(out));
		};
	"#;
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["hasFoe"], true, "FIND_HOSTILE_CREEPS 应找到敌人: {info}");
	assert_eq!(info["foeMy"], false, "敌人 my=false: {info}");
	assert_eq!(info["attack"], 0, "attack 邻格敌人应 OK: {info}");
	assert_eq!(info["rangedAttack"], 0, "rangedAttack 应 OK: {info}");
	assert_eq!(info["rangedMassAttack"], 0, "rangedMassAttack 应 OK: {info}");
	assert_eq!(info["healSelf"], 0, "heal 邻格自身应 OK: {info}");
	assert_eq!(info["rangedHealSelf"], 0, "rangedHeal 应 OK: {info}");
	assert_eq!(info["dismantleWall"], -9, "dismantle 2 格外墙 ERR_NOT_IN_RANGE: {info}");
	assert_eq!(info["sign"], -9, "signController 8 格外 ERR_NOT_IN_RANGE: {info}");
	assert_eq!(info["attackControllerNeutral"], -9, "官方顺序: 距离检查先于 owner 检查, 8 格外 ERR_NOT_IN_RANGE: {info}");
	let range = info["ctrlRange"].as_i64().unwrap();
	assert!(range >= 4, "ally 与控制器距离应 >3: {info}");
	assert_eq!(info["claimFar"], -9, "claimController 超距 ERR_NOT_IN_RANGE: {info}");
	assert_eq!(info["reserveFar"], -9, "reserveController 超距 ERR_NOT_IN_RANGE: {info}");
	// 意图应被提交
	let names: Vec<&str> = entries.iter().map(|(_, n, _)| n.as_str()).collect();
	for expect in ["attack", "rangedAttack", "rangedMassAttack", "heal", "rangedHeal"] {
		assert!(names.contains(&expect), "应提交 {expect} 意图, 实际 {names:?}");
	}
}

/// 战斗意图经引擎结算 (官方互斥表: heal 压制 attack/rangedAttack/rangedMassAttack;
/// rangedMassAttack 压制 rangedAttack)。只提交不冲突组合 attack + rangedMassAttack:
/// Foe -30-10=-40; Ally 被 Foe 1×ATTACK 反击 -30
#[test]
fn combat_intents_resolve_through_engine() {
	in_tokio(|| combat_intents_resolve_through_engine_inner());
}

fn combat_intents_resolve_through_engine_inner() {
	let scenario = Scenario::from_json(COMBAT_SCENE).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const ally = Game.creeps.Ally;
			const foe = ally.room.find(FIND_HOSTILE_CREEPS)[0];
			ally.attack(foe);
			ally.rangedMassAttack();
			console.log(JSON.stringify({ ok: true }));
		};
	"#;
	let (entries, _) = run_once(bundle, &mut world);
	let movement = &mut Movement::default();
	tick::run_tick(&mut world, movement, &IntentList { entries });
	let get_hits = |name: &str| -> u32 {
		world.rooms["W5N8"]
			.objects
			.values()
			.find(|o| o.name.as_deref() == Some(name))
			.map(|o| o.hits)
			.unwrap_or(0)
	};
	// attack: 1×30; rangedMassAttack: 距离 1 → 10×1.0; 两者互不压制
	assert_eq!(get_hits("Foe"), 200 - 40, "attack 30 + rangedMassAttack 10");
	// 近战触发反击: Foe 1×ATTACK × 30
	assert_eq!(get_hits("Ally"), 700 - 30, "被反击 30");
}

/// 官方互斥表 (checkPriorities): heal 与 attack 同 tick 提交 → 只有 heal 执行
#[test]
fn heal_suppresses_attack_same_tick_via_js() {
	in_tokio(|| heal_suppresses_attack_same_tick_via_js_inner());
}

fn heal_suppresses_attack_same_tick_via_js_inner() {
	let scenario = Scenario::from_json(COMBAT_SCENE).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const ally = Game.creeps.Ally;
			const foe = ally.room.find(FIND_HOSTILE_CREEPS)[0];
			ally.attack(foe);
			ally.heal(ally);
			console.log(JSON.stringify({ ok: true }));
		};
	"#;
	let (entries, _) = run_once(bundle, &mut world);
	let movement = &mut Movement::default();
	tick::run_tick(&mut world, movement, &IntentList { entries });
	let foe_hits = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.name.as_deref() == Some("Foe"))
		.map(|o| o.hits)
		.unwrap_or(0);
	assert_eq!(foe_hits, 200, "heal 在场时 attack 被互斥表压制 (官方 checkPriorities)");
}

/// 中立控制器: claim → 归属我方 (经 V8 提交意图)
#[test]
fn claim_controller_via_js() {
	in_tokio(|| claim_controller_via_js_inner());
}

fn claim_controller_via_js_inner() {
	let scenario = Scenario::from_json(COMBAT_SCENE).unwrap();
	let mut world = build_world(&scenario).unwrap();
	// 把 Ally 移到控制器旁
	{
		let ctrl = world.rooms["W5N8"].objects.values().find(|o| o.obj_type == "controller").unwrap();
		let (cx, cy) = (ctrl.x, ctrl.y);
		let ally = world
			.rooms
			.get_mut("W5N8")
			.unwrap()
			.objects
			.values_mut()
			.find(|o| o.name.as_deref() == Some("Ally"))
			.unwrap();
		ally.x = cx;
		ally.y = cy + 1;
	}
	let bundle = r#"
		exports.loop = function() {
			const ally = Game.creeps.Ally;
			const ctrl = ally.room.controller;
			console.log(JSON.stringify({ code: ally.claimController(ctrl), my: ctrl.my, level: ctrl.level }));
		};
	"#;
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["code"], 0, "claimController 应 OK: {info}");
	let movement = &mut Movement::default();
	tick::run_tick(&mut world, movement, &IntentList { entries });
	let ctrl = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap();
	assert!(ctrl.mine, "claim 后控制器应归属我方");
	assert_eq!(ctrl.level, 1);
}

/// loop() 抛异常: 异常前已登记的意图与 Memory 写入照常保存并结算 (官方 runner.js 语义)
#[test]
fn loop_exception_keeps_intents_and_memory() {
	in_tokio(|| loop_exception_keeps_intents_and_memory_inner());
}

fn loop_exception_keeps_intents_and_memory_inner() {
	let scenario = Scenario::from_json(TWO_ROOMS).unwrap();
	let mut world = build_world(&scenario).unwrap();
	use nexus_sim::engine::objects::{creep_obj, BodyPart};
	let id = nexus_sim::engine::world::alloc_id(&mut world.next_id);
	let body: Vec<BodyPart> = ["move"]
		.iter()
		.map(|p| BodyPart { part_type: p.to_string(), hits: 100 })
		.collect();
	world
		.rooms
		.get_mut("W5N8")
		.unwrap()
		.objects
		.insert(id, creep_obj(id, "E1", 25, 25, body));

	let bundle = r#"
		exports.loop = function() {
			Memory.before = true;
			const creep = Object.values(Game.creeps)[0];
			creep.move(RIGHT);
			throw new Error("boom");
		};
	"#;
	let snapshot = build_snapshot(
		&world,
		&SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: false,
		},
	);
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let result = bot
		.run_tick(&serde_json::to_string(&snapshot).unwrap(), "{}")
		.unwrap();
	assert_eq!(result.errors.len(), 1, "异常应被捕获并上报 (不中断模拟)");
	assert_eq!(result.errors[0].contains("boom"), true, "异常信息应包含原始消息");
	assert!(
		result.memory.contains("\"before\":true"),
		"异常前的 Memory 写入应保留: {}",
		result.memory
	);
	let intents = IntentList::parse(&result.intents_json);
	assert!(
		intents.entries.iter().any(|(_, n, _)| n == "move"),
		"异常前登记的意图不应丢弃"
	);
	let movement = &mut Movement::default();
	tick::run_tick(&mut world, movement, &intents);
	assert_eq!(world.rooms["W5N8"].objects[&id].x, 26, "已登记意图应真实生效");
}

