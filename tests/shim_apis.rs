//! JS 侧机制测试: 用微型 bundle (非 bot) 直接驱动 V8 + shim, 验证快照/寻路/意图回收链路。

use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::snapshot::{build_snapshot, CpuView, SnapshotOptions};
use screeps_sim::engine::tick;
use screeps_sim::engine::intents::IntentList;
use screeps_sim::runtime::BotRuntime;

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

fn run_once(bundle: &str, world: &mut screeps_sim::engine::world::World) -> (Vec<(String, String, serde_json::Value)>, Vec<String>) {
	let snapshot = build_snapshot(
		world,
		&SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: false,
			include_terrain: true,
		},
	);
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let result = bot.run_tick(snapshot, "{}".to_string()).unwrap();
	(result.intents.entries, result.console)
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
	use screeps_sim::engine::objects::{creep_obj, BodyPart};
	let id = screeps_sim::engine::world::alloc_id(&mut world.next_id);
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
			include_terrain: true,
			},
		);
		let result = bot
			.run_tick(snapshot, memory.clone())
			.unwrap();
		memory = result.memory.clone();
		let info: serde_json::Value =
			serde_json::from_str(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info["code"], 0, "moveTo 应返回 OK: {info}");
		// Memory 首 tick 写入后应跨 tick 保留 (此前每 tick 都新建 runtime 且传 "{}", 测不到)
		assert_eq!(info["started"], 0, "Memory 应跨 tick 往返: {info}");

		let intents = result.intents;
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
	use screeps_sim::engine::objects::{creep_obj, BodyPart};
	let id = screeps_sim::engine::world::alloc_id(&mut world.next_id);
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
			include_terrain: true,
		},
	);
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let result = bot
		.run_tick(snapshot, "{}".to_string())
		.unwrap();
	assert_eq!(result.errors.len(), 1, "异常应被捕获并上报 (不中断模拟)");
	assert_eq!(result.errors[0].contains("boom"), true, "异常信息应包含原始消息");
	assert!(
		result.memory.contains("\"before\":true"),
		"异常前的 Memory 写入应保留: {}",
		result.memory
	);
	let intents = result.intents;
	assert!(
		intents.entries.iter().any(|(_, n, _)| n == "move"),
		"异常前登记的意图不应丢弃"
	);
	let movement = &mut Movement::default();
	tick::run_tick(&mut world, movement, &intents);
	assert_eq!(world.rooms["W5N8"].objects[&id].x, 26, "已登记意图应真实生效");
}


// ═════════════════════════ 转移: 正在孵化的 spawn 仍可收货 ═════════════════════════

/// 回归: shim 曾把 `target.spawning` 当成「目标是正在孵化的 creep」, 而 spawn 孵化时 `spawning` 是
/// Spawning 对象 (truthy), 导致孵化期间所有 transfer(spawn) 返回 ERR_INVALID_TARGET。
/// 官方 creeps.js:445 只拒绝 `target instanceof Creep && target.spawning`。真实 bot 里表现为
/// spawn 一直在孵化 → 搬运工送不进能量 → 房间能量卡在 300、升级停摆数千 tick。
#[test]
fn transfer_into_busy_spawn() {
	in_tokio(|| transfer_into_busy_spawn_inner());
}

fn transfer_into_busy_spawn_inner() {
	let scenario = Scenario::from_json(
		r#"{
		"name": "busy-spawn", "seed": 1,
		"rooms": [{ "name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 2, "pos": [28, 27] },
			"structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 300 },
				{ "type": "extension", "pos": [22, 22], "energy": 0 }
			],
			"creeps": [{ "name": "Hauler", "body": ["carry", "carry", "move"], "pos": [26, 25], "store": { "energy": 100 } }]
		}]
	}"#,
	)
	.unwrap();
	let mut world = build_world(&scenario).unwrap();
	let mut movement = Movement::default();
	let bundle = r#"
		exports.loop = function() {
			const spawn = Game.spawns[Object.keys(Game.spawns)[0]];
			const hauler = Game.creeps.Hauler;
			if (!spawn.spawning) {
				console.log(JSON.stringify({ phase: "spawn", code: spawn.spawnCreep([WORK, WORK, CARRY, MOVE], "Kid") }));
				return;
			}
			const ext = spawn.room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_EXTENSION } })[0];
			console.log(JSON.stringify({
				phase: "transfer",
				spawnEnergy: spawn.store[RESOURCE_ENERGY],
				free: spawn.store.getFreeCapacity(RESOURCE_ENERGY),
				cap: spawn.store.getCapacity(),
				extCap: ext.store.getCapacity(RESOURCE_ENERGY),
				extFree: ext.store.getFreeCapacity(RESOURCE_ENERGY),
				ghost: spawn.store.getCapacity(RESOURCE_HYDROGEN),
				badRes: hauler.transfer(spawn, RESOURCE_HYDROGEN),
				code: hauler.transfer(spawn, RESOURCE_ENERGY),
			}));
		};
	"#;
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let mut memory = String::from("{}");
	let mut infos = Vec::new();
	for _ in 0..2 {
		let snapshot = build_snapshot(
			&world,
			&SnapshotOptions {
				cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
				always_visible: false,
				include_terrain: true,
			},
		);
		let result = bot.run_tick(snapshot, memory.clone()).unwrap();
		memory = result.memory.clone();
		infos.push(serde_json::from_str::<serde_json::Value>(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap());
		tick::run_tick(&mut world, &mut movement, &result.intents);
	}
	assert_eq!(infos[0]["code"], 0, "spawnCreep 应 OK: {}", infos[0]);
	let t = &infos[1];
	assert_eq!(t["phase"], "transfer", "第二 tick spawn 应在孵化: {t}");
	assert_eq!(t["spawnEnergy"], 1, "孵化扣掉 300, 再按官方自动回能 +1: {t}");
	assert_eq!(t["free"], 299, "{t}");
	assert_eq!(t["cap"], serde_json::Value::Null, "专用仓 getCapacity() 为 null: {t}");
	assert_eq!(t["extCap"], 50, "{t}");
	assert_eq!(t["extFree"], 50, "{t}");
	assert_eq!(t["ghost"], serde_json::Value::Null, "spawn 不收 H: {t}");
	assert_eq!(t["badRes"], -7, "非能量进 spawn → ERR_INVALID_TARGET (官方 capacityForResource 为 0): {t}");
	assert_eq!(t["code"], 0, "正在孵化的 spawn 应能收货: {t}");
	let spawn_energy = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "spawn")
		.and_then(|o| o.store.get("energy").copied())
		.unwrap_or(0);
	assert_eq!(spawn_energy, 101 + 1, "能量应进 spawn (1 + 100, 本 tick 房间能量仍 < 300 再回 1)");
}

// ═════════════════════════ isActive / ERR_RCL_NOT_ENOUGH (官方 checkStructureAgainstController) ═════════════════════════

/// RCL3 塔上限 1: 离控制器近的塔 isActive()=true, 远的 false; 远塔 attack/heal/repair 在能量检查之后
/// 返回 ERR_RCL_NOT_ENOUGH; 超限的 extension / 无主的路同样按官方规则判定
#[test]
fn is_active_follows_controller_structures() {
	in_tokio(|| is_active_follows_controller_structures_inner());
}

fn is_active_follows_controller_structures_inner() {
	let scenario = Scenario::from_json(
		r#"{
		"name": "active", "seed": 1,
		"rooms": [{ "name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 3, "pos": [28, 27] },
			"structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 300 },
				{ "type": "tower", "pos": [40, 40], "energy": 500 },
				{ "type": "tower", "pos": [30, 27], "energy": 500 },
				{ "type": "tower", "pos": [41, 41], "energy": 5 },
				{ "type": "road", "pos": [10, 10] },
				{ "type": "constructedWall", "pos": [11, 11] }
			],
			"creeps": [
				{ "name": "Me", "body": ["move"], "pos": [26, 26] },
				{ "name": "Foe", "body": ["move"], "pos": [35, 35], "enemy": true }
			]
		}]
	}"#,
	)
	.unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const room = Game.rooms.W5N8;
			const towers = room.find(FIND_MY_STRUCTURES, { filter: { structureType: STRUCTURE_TOWER } });
			const at = (x, y) => towers.find(t => t.pos.x === x && t.pos.y === y);
			const near = at(30, 27), far = at(40, 40), poor = at(41, 41);
			const foe = room.find(FIND_HOSTILE_CREEPS)[0];
			const road = room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_ROAD } })[0];
			const wall = room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_WALL } })[0];
			console.log(JSON.stringify({
				nearActive: near.isActive(), farActive: far.isActive(), poorActive: poor.isActive(),
				roadActive: road.isActive(), spawnActive: Game.spawns[Object.keys(Game.spawns)[0]].isActive(),
				ctrlActive: room.controller.isActive(),
				nearAttack: near.attack(foe),
				farAttack: far.attack(foe), farHeal: far.heal(Game.creeps.Me), farRepair: far.repair(wall),
				poorAttack: poor.attack(foe),
			}));
		};
	"#;
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["nearActive"], true, "{info}");
	assert_eq!(info["farActive"], false, "{info}");
	assert_eq!(info["poorActive"], false, "{info}");
	assert_eq!(info["roadActive"], true, "无主结构恒 true: {info}");
	assert_eq!(info["spawnActive"], true, "{info}");
	assert_eq!(info["ctrlActive"], true, "controller 不在 CONTROLLER_STRUCTURES 表中: {info}");
	assert_eq!(info["nearAttack"], 0, "{info}");
	assert_eq!(info["farAttack"], -14, "远塔 ERR_RCL_NOT_ENOUGH: {info}");
	assert_eq!(info["farHeal"], -14, "{info}");
	assert_eq!(info["farRepair"], -14, "{info}");
	assert_eq!(info["poorAttack"], -6, "能量检查先于 RCL 检查 → ERR_NOT_ENOUGH_ENERGY: {info}");
	assert_eq!(entries.iter().filter(|(_, n, _)| n == "attack").count(), 1, "只有近塔提交 attack 意图");
}

// ═════════════════════════ StructureSpawn.Spawning / spawnCreep 官方语义 ═════════════════════════

fn snapshot_of(world: &screeps_sim::engine::world::World) -> String {
	build_snapshot(
		world,
		&SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: false,
			include_terrain: true,
		},
	)
}

/// spawnCreep: 占位 creep 立即进 Game.creeps; Memory 合并规则; 校验顺序 (重名先于 busy、directions、dryRun);
/// 下一 tick spawn.spawning 是 StructureSpawn.Spawning (spawn 不可枚举), setDirections / cancel 提交官方意图
#[test]
fn spawning_object_and_spawn_creep_semantics() {
	in_tokio(|| spawning_object_and_spawn_creep_semantics_inner());
}

fn spawning_object_and_spawn_creep_semantics_inner() {
	let scenario = Scenario::from_json(TWO_ROOMS).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let mut movement = Movement::default();
	let bundle = r#"
		exports.loop = function() {
			const spawn = Game.spawns[Object.keys(Game.spawns)[0]];
			if (!spawn.spawning) {
				Memory.creeps = { Old: { keep: 1 } };
				const out = {};
				out.badDirs = spawn.spawnCreep([MOVE], "A", { directions: [1, 9] });
				out.emptyDirs = spawn.spawnCreep([MOVE], "A", { directions: [] });
				out.notArray = spawn.spawnCreep([MOVE], "A", { directions: 3 });
				out.noName = spawn.spawnCreep([MOVE]);
				out.nullOpts = spawn.spawnCreep([MOVE], "A", null);
				out.badPart = spawn.spawnCreep(["wing"], "A");
				out.esDup = spawn.spawnCreep([WORK, WORK, MOVE, MOVE, MOVE], "A", { energyStructures: [spawn, spawn], dryRun: true });
				out.esStrIds = spawn.spawnCreep([MOVE], "A", { energyStructures: [spawn.id], dryRun: true });
				out.dry = spawn.spawnCreep([MOVE], "A", { dryRun: true });
				out.dryMem = Memory.creeps.A === undefined && !Game.creeps.A;
				out.ok = spawn.spawnCreep([CARRY, CARRY, MOVE], "Old", { directions: ["5", 5, 3] });
				const c = Game.creeps.Old;
				out.ph = c && { spawning: c.spawning, my: c.my, ttl: c.ticksToLive, hits: c.hits, hitsMax: c.hitsMax,
					cap: c.carryCapacity, used: c.store.getUsedCapacity(), fatigue: c.fatigue, owner: c.owner.username,
					x: c.pos.x, y: c.pos.y, body: c.body.map(p => p.type + p.hits).join(), isCreep: c instanceof Creep };
				out.mem = Memory.creeps.Old;
				out.nameExists = spawn.spawnCreep([MOVE], "Old");
				out.nameFirst = spawn.spawnCreep([MOVE], "Old", { directions: [0] });
				out.numName = spawn.spawnCreep([MOVE], 12345, { dryRun: true });
				console.log(JSON.stringify(out));
				return;
			}
			const sp = spawn.spawning;
			console.log(JSON.stringify({
				isSpawning: sp instanceof StructureSpawn.Spawning,
				keys: Object.keys(sp).sort().join(),
				spawnIsSelf: sp.spawn === spawn,
				name: sp.name, needTime: sp.needTime, remainingTime: sp.remainingTime, directions: sp.directions,
				badSet: sp.setDirections([0]), emptySet: sp.setDirections([]), notArraySet: sp.setDirections(5),
				okSet: sp.setDirections([7, "7", 1]),
				nameBeforeBusy: spawn.spawnCreep([MOVE], "Old"),
				busy: spawn.spawnCreep([MOVE], "B"),
				cancel: sp.cancel(),
			}));
		};
	"#;
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let r1 = bot.run_tick(snapshot_of(&world), "{}".to_string()).unwrap();
	let a: serde_json::Value = serde_json::from_str(&r1.console[0]).unwrap();
	assert_eq!(a["badDirs"], -10, "{a}");
	assert_eq!(a["emptyDirs"], -10, "{a}");
	assert_eq!(a["notArray"], -10, "{a}");
	assert_eq!(a["noName"], -10, "{a}");
	assert_eq!(a["nullOpts"], -10, "{a}");
	assert_eq!(a["badPart"], -10, "{a}");
	assert_eq!(a["esDup"], -6, "两次列同一 spawn 只算一次 (300 < 350): {a}");
	assert_eq!(a["esStrIds"], -6, "官方 _.map(es, 'id'): 传 id 字符串得不到能量: {a}");
	assert_eq!(a["dry"], 0, "{a}");
	assert_eq!(a["dryMem"], true, "dryRun 无副作用: {a}");
	assert_eq!(a["ok"], 0, "{a}");
	let ph = &a["ph"];
	assert_eq!(ph["spawning"], true, "{a}");
	assert_eq!(ph["my"], true);
	assert_eq!(ph["ttl"], 1500);
	assert_eq!(ph["hits"], 300);
	assert_eq!(ph["hitsMax"], 300);
	assert_eq!(ph["cap"], 100);
	assert_eq!(ph["used"], 0);
	assert_eq!(ph["fatigue"], 0);
	assert_eq!(ph["owner"], "Nexus");
	assert_eq!((ph["x"].as_u64(), ph["y"].as_u64()), (Some(25), Some(25)), "占位 creep 在 spawn 格");
	assert_eq!(ph["body"], "carry100,carry100,move100");
	assert_eq!(ph["isCreep"], true);
	assert_eq!(a["mem"], serde_json::json!({ "keep": 1 }), "未给 opts.memory 时保留已有 Memory.creeps[name]: {a}");
	assert_eq!(a["nameExists"], -3, "{a}");
	assert_eq!(a["nameFirst"], -3, "重名检查先于 directions 校验: {a}");
	assert_eq!(a["numName"], 0, "name.toString(): {a}");
	let spawn_intents: Vec<_> = r1.intents.entries.iter().filter(|(_, n, _)| n == "spawnCreep").collect();
	assert_eq!(spawn_intents.len(), 1, "只有一个 spawnCreep 意图");
	assert_eq!(spawn_intents[0].2["directions"], serde_json::json!([5, 3]), "directions 转数字去重");
	tick::run_tick(&mut world, &mut movement, &r1.intents);

	let r2 = bot.run_tick(snapshot_of(&world), r1.memory.clone()).unwrap();
	let b: serde_json::Value = serde_json::from_str(&r2.console[0]).unwrap();
	assert_eq!(b["isSpawning"], true, "{b}");
	assert_eq!(b["keys"], "directions,name,needTime,remainingTime", "spawn 不可枚举: {b}");
	assert_eq!(b["spawnIsSelf"], true, "{b}");
	assert_eq!(b["name"], "Old");
	assert_eq!(b["needTime"], 9);
	assert_eq!(b["remainingTime"], 8);
	assert_eq!(b["directions"], serde_json::json!([5, 3]), "快照下发 directions: {b}");
	assert_eq!(b["badSet"], -10);
	assert_eq!(b["emptySet"], -10);
	assert_eq!(b["notArraySet"], -10);
	assert_eq!(b["okSet"], 0);
	assert_eq!(b["nameBeforeBusy"], -3, "官方: 重名检查先于 ERR_BUSY: {b}");
	assert_eq!(b["busy"], -4, "{b}");
	assert_eq!(b["cancel"], 0);
	let names: Vec<&str> = r2.intents.entries.iter().map(|(_, n, _)| n.as_str()).collect();
	assert!(names.contains(&"setSpawnDirections") && names.contains(&"cancelSpawning"), "{names:?}");
	let set = r2.intents.entries.iter().find(|(_, n, _)| n == "setSpawnDirections").unwrap();
	assert_eq!(set.2["directions"], serde_json::json!([7, 1]));
	tick::run_tick(&mut world, &mut movement, &r2.intents);
	assert!(
		!world.rooms["W5N8"].objects.values().any(|o| o.obj_type == "creep"),
		"cancel 后孵化中的 creep 被移除"
	);
	assert!(world.rooms["W5N8"].objects.values().find(|o| o.obj_type == "spawn").unwrap().spawn_job.is_none());
}

// ═════════════════════════ renewCreep / link.transferEnergy 返回码 ═════════════════════════

/// renewCreep: CLAIM creep → ERR_INVALID_TARGET。link.transferEnergy: 显式 amount 超过目标剩余 → ERR_FULL;
/// 跨房 → ERR_NOT_IN_RANGE; 超出 RCL 上限 → ERR_RCL_NOT_ENOUGH; 缺省 amount 自动取 min(自身, 目标剩余)
#[test]
fn renew_claim_and_link_transfer_codes() {
	in_tokio(|| renew_claim_and_link_transfer_codes_inner());
}

fn renew_claim_and_link_transfer_codes_inner() {
	let scenario = Scenario::from_json(
		r#"{
		"name": "links", "seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "controller": { "level": 5, "pos": [28, 27] },
			  "structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 300 },
				{ "type": "link", "pos": [30, 30], "energy": 500 },
				{ "type": "link", "pos": [32, 30], "energy": 700 },
				{ "type": "link", "pos": [45, 45], "energy": 500 }
			  ],
			  "creeps": [{ "name": "Claimer", "body": ["claim", "move"], "pos": [26, 25] }] },
			{ "name": "W5N7", "terrain": { "kind": "flat" },
			  "structures": [{ "type": "link", "pos": [10, 10], "energy": 0 }] }
		]
	}"#,
	)
	.unwrap();
	let mut world = build_world(&scenario).unwrap();
	let bundle = r#"
		exports.loop = function() {
			const spawn = Game.spawns[Object.keys(Game.spawns)[0]];
			const links = Game.rooms.W5N8.find(FIND_MY_STRUCTURES, { filter: { structureType: STRUCTURE_LINK } });
			const at = (x) => links.find(l => l.pos.x === x);
			const a = at(30), b = at(32), far = at(45);
			const other = Game.rooms.W5N7.find(FIND_MY_STRUCTURES)[0];
			console.log(JSON.stringify({
				renewClaim: spawn.renewCreep(Game.creeps.Claimer),
				full: a.transferEnergy(b, 200),
				farActive: far.isActive(),
				rcl: far.transferEnergy(b),
				cross: a.transferEnergy(other, 100),
				auto: a.transferEnergy(b),
			}));
		};
	"#;
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	assert_eq!(info["renewClaim"], -7, "{info}");
	assert_eq!(info["full"], -8, "700 + 200 > 800 → ERR_FULL: {info}");
	assert_eq!(info["farActive"], false, "RCL5 只有 2 个 link 名额, 最远的停用: {info}");
	assert_eq!(info["rcl"], -14, "{info}");
	assert_eq!(info["cross"], -9, "跨房 ERR_NOT_IN_RANGE: {info}");
	assert_eq!(info["auto"], 0, "{info}");
	let t = entries.iter().find(|(_, n, _)| n == "transferEnergy").expect("应提交 transferEnergy");
	assert_eq!(t.2["amount"], 100, "缺省 amount = min(500, 800-700)");
	assert_eq!(entries.iter().filter(|(_, n, _)| n == "transferEnergy" || n == "renewCreep").count(), 1);
}

// ═════════════════════════ Structure.destroy / Ruin ═════════════════════════

/// destroy(): 看控制器归属 (不是结构归属), 房内有敌方 creep → ERR_BUSY; 摧毁后下一 tick 出现 Ruin
#[test]
fn destroy_and_ruin_object() {
	in_tokio(|| destroy_and_ruin_object_inner());
}

fn destroy_and_ruin_object_inner() {
	let scene = |enemy: bool| {
		format!(
			r#"{{
		"name": "ruin", "seed": 1,
		"rooms": [{{ "name": "W5N8", "terrain": {{ "kind": "flat" }},
			"controller": {{ "level": 3, "pos": [28, 27] }},
			"structures": [
				{{ "type": "spawn", "pos": [25, 25], "energy": 300 }},
				{{ "type": "tower", "pos": [30, 30], "energy": 250 }},
				{{ "type": "road", "pos": [10, 10] }}
			],
			"creeps": [{{ "name": "Foe", "body": ["move"], "pos": [40, 40], "enemy": {enemy} }}]
		}}]
	}}"#
		)
	};
	let bundle = r#"
		exports.loop = function() {
			const room = Game.rooms.W5N8;
			// FIND_RUINS 尚未接入 (find/look 由另一分支改动), 这里直接扫房间对象
			const ruins = room.__objects.filter(o => o instanceof Ruin);
			if (ruins.length) {
				const desc = (r) => ({
					st: r.structureType, type: r.structure.structureType, sid: r.structure.id,
					hits: r.structure.hits, hitsMax: r.structure.hitsMax, my: r.structure.my,
					owned: r.structure instanceof OwnedStructure, isStructure: r.structure instanceof Structure,
					energy: r.store[RESOURCE_ENERGY], cap: r.store.getCapacity(), ttd: r.ticksToDecay,
					destroyTime: r.destroyTime, byId: Game.getObjectById(r.id) === r, str: String(r),
				});
				const t = ruins.find(r => r.structure.structureType === "tower");
				const rd = ruins.find(r => r.structure.structureType === "road");
				console.log(JSON.stringify({ phase: "ruin", tower: desc(t), road: desc(rd), towerId: Memory.towerId }));
				return;
			}
			const tower = room.find(FIND_MY_STRUCTURES, { filter: { structureType: STRUCTURE_TOWER } })[0];
			const road = room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_ROAD } })[0];
			Memory.towerId = tower.id;
			console.log(JSON.stringify({ phase: "destroy", road: road.destroy(), tower: tower.destroy() }));
		};
	"#;
	// 有敌方 creep: ERR_BUSY
	let mut world = build_world(&Scenario::from_json(&scene(true)).unwrap()).unwrap();
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(&console[0]).unwrap();
	assert_eq!(info["tower"], -4, "{info}");
	assert_eq!(info["road"], -4, "{info}");
	assert!(entries.is_empty());
	// 无敌方: 无主的路也能拆 (官方只看控制器归属)
	let mut world = build_world(&Scenario::from_json(&scene(false)).unwrap()).unwrap();
	let mut movement = Movement::default();
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let r1 = bot.run_tick(snapshot_of(&world), "{}".to_string()).unwrap();
	let info: serde_json::Value = serde_json::from_str(&r1.console[0]).unwrap();
	assert_eq!((info["road"].as_i64(), info["tower"].as_i64()), (Some(0), Some(0)), "{info}");
	tick::run_tick(&mut world, &mut movement, &r1.intents);
	let r2 = bot.run_tick(snapshot_of(&world), r1.memory.clone()).unwrap();
	let all: serde_json::Value = serde_json::from_str(&r2.console[0]).unwrap();
	assert_eq!(all["phase"], "ruin", "{all}");
	let t = &all["tower"];
	assert_eq!(t["st"], serde_json::Value::Null, "Ruin.structureType 为 undefined: {all}");
	assert_eq!(t["type"], "tower");
	assert_eq!(t["sid"], all["towerId"], "structure.id 是原结构 id: {all}");
	assert_eq!(t["hits"], 0);
	assert_eq!(t["hitsMax"], 3000);
	assert_eq!(t["my"], true);
	assert_eq!(t["owned"], true, "有主结构 → OwnedStructure: {all}");
	assert_eq!(t["isStructure"], true);
	assert_eq!(t["energy"], 250, "仓储留在 ruin: {all}");
	assert_eq!(t["cap"], serde_json::Value::Null, "Ruin store 无容量: {all}");
	assert_eq!(t["ttd"], 499, "decayTime(0+500) - time(1): {all}");
	assert_eq!(t["destroyTime"], 0);
	assert_eq!(t["byId"], true);
	assert!(t["str"].as_str().unwrap().starts_with("[ruin (tower) #"), "{all}");
	let r = &all["road"];
	assert_eq!(r["my"], serde_json::Value::Null, "无主结构的 ruin.structure 无 my: {all}");
	assert_eq!(r["owned"], false);
	assert_eq!(r["isStructure"], true);
}

// ═════════════════════════ rampart / road 属性 ═════════════════════════

/// rampart.isPublic 来自快照、setPublic 提交意图; road.ticksToDecay 不再被强制为 undefined
#[test]
fn rampart_public_and_road_decay_exposed() {
	in_tokio(|| rampart_public_and_road_decay_exposed_inner());
}

fn rampart_public_and_road_decay_exposed_inner() {
	let scenario = Scenario::from_json(
		r#"{
		"name": "rampart", "seed": 1,
		"rooms": [{ "name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 3, "pos": [28, 27] },
			"structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 300 },
				{ "type": "rampart", "pos": [20, 20], "hits": 5000 },
				{ "type": "road", "pos": [10, 10] }
			] }]
	}"#,
	)
	.unwrap();
	let mut world = build_world(&scenario).unwrap();
	for o in world.rooms.get_mut("W5N8").unwrap().objects.values_mut() {
		if o.obj_type == "rampart" {
			o.is_public = true;
		}
	}
	let bundle = r#"
		exports.loop = function() {
			const room = Game.rooms.W5N8;
			const rampart = room.find(FIND_MY_STRUCTURES, { filter: { structureType: STRUCTURE_RAMPART } })[0];
			const road = room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_ROAD } })[0];
			console.log(JSON.stringify({
				isPublic: rampart.isPublic, my: rampart.my, roadTtd: road.ticksToDecay, rampartTtd: rampart.ticksToDecay,
				set: rampart.setPublic(0),
			}));
		};
	"#;
	let (entries, console) = run_once(bundle, &mut world);
	let info: serde_json::Value = serde_json::from_str(&console[0]).unwrap();
	assert_eq!(info["isPublic"], true, "{info}");
	assert_eq!(info["my"], true, "{info}");
	assert_eq!(info["roadTtd"], 1000, "{info}");
	assert_eq!(info["rampartTtd"], 100, "{info}");
	assert_eq!(info["set"], 0, "{info}");
	let e = entries.iter().find(|(_, n, _)| n == "setPublic").expect("setPublic 意图");
	assert_eq!(e.2["isPublic"], false, "!!0 → false");
}

/// reserveController: 自己的保留可以续 (此前 shim 用 !target.my 判断, 第二次起恒返回 ERR_INVALID_TARGET);
/// reservation 字段名是 ticksToEnd
#[test]
fn reserve_controller_renews_own_reservation() {
	in_tokio(|| {
		let scenario = Scenario::from_json(
			r#"{ "name": "r", "seed": 1, "rooms": [{ "name": "W5N8", "terrain": { "kind": "flat" },
				"controller": { "level": 0, "pos": [28, 27], "mine": false },
				"creeps": [{ "name": "R", "body": ["claim", "move"], "pos": [27, 27] }] }] }"#,
		)
		.unwrap();
		let mut world = build_world(&scenario).unwrap();
		let mut movement = Movement::default();
		let bundle = r#"exports.loop = function() {
			const c = Game.creeps.R; const ctrl = c.room.controller;
			console.log(JSON.stringify({ code: c.reserveController(ctrl), res: ctrl.reservation === undefined ? "none" : ctrl.reservation }));
		};"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let mut memory = String::from("{}");
		let mut out = Vec::new();
		for _ in 0..3 {
			let snapshot = build_snapshot(&world, &SnapshotOptions { cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 }, always_visible: false, include_terrain: true });
			let r = bot.run_tick(snapshot, memory.clone()).unwrap();
			memory = r.memory.clone();
			out.push(serde_json::from_str::<serde_json::Value>(&r.console[0]).unwrap());
			tick::run_tick(&mut world, &mut movement, &r.intents);
		}
		assert_eq!(out[0]["res"], "none", "{out:?}");
		assert!(out.iter().all(|o| o["code"] == 0), "每 tick 都能续: {out:?}");
		assert_eq!(out[2]["res"]["username"], "Nexus", "{out:?}");
		assert!(out[2]["res"]["ticksToEnd"].as_u64().unwrap() >= 1, "ticksToEnd 字段: {out:?}");
	});
}

/// Creep API 返回码顺序对齐官方 creeps.js: withdraw / pickup / upgradeController / claimController / cancelOrder / say
#[test]
fn creep_api_return_codes_match_official() {
	in_tokio(|| {
		let scenario = Scenario::from_json(
			r#"{ "name": "codes", "seed": 1, "rooms": [
				{ "name": "W5N8", "terrain": { "kind": "flat" },
				  "controller": { "level": 2, "pos": [40, 40] },
				  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }, { "type": "container", "pos": [10, 10] }],
				  "creeps": [
					{ "name": "Full", "body": ["carry", "move"], "pos": [30, 30], "store": { "energy": 50 } },
					{ "name": "Empty", "body": ["carry", "work", "claim", "move"], "pos": [20, 20] },
					{ "name": "Other", "body": ["carry", "move"], "pos": [21, 20], "store": { "energy": 50 } }
				  ] },
				{ "name": "W5N9", "terrain": { "kind": "flat" }, "controller": { "level": 0, "pos": [20, 20], "mine": false } }
			] }"#,
		)
		.unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"exports.loop = function() {
			const full = Game.creeps.Full, empty = Game.creeps.Empty, other = Game.creeps.Other;
			const container = empty.room.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_CONTAINER } })[0];
			const spawn = Game.spawns[Object.keys(Game.spawns)[0]];
			const r = {
				// 空容器在远处: 官方先查距离 → ERR_NOT_IN_RANGE (此前 shim 先报 ERR_NOT_ENOUGH_RESOURCES)
				withdrawFarEmpty: empty.withdraw(container, RESOURCE_ENERGY),
				// 不能从别的 creep 身上 withdraw
				withdrawCreep: empty.withdraw(other, RESOURCE_ENERGY),
				// 显式数量超过目标存量
				withdrawTooMuch: full.withdraw(spawn, RESOURCE_ENERGY, 10),
				// 满载 creep 远处 pickup: 官方先判满 → ERR_FULL
				pickupFull: full.pickup(new Resource({ id: "ffff", x: 1, y: 1, roomName: "W5N8", energy: 10, resourceType: "energy" })),
				say: full.say(123),
				// 已占 1 房、GCL 1: 官方第一步就是 GCL 检查 → ERR_GCL_NOT_ENOUGH
				claimGcl: empty.claimController(empty.room.controller),
				cancelMissing: full.cancelOrder("move"),
				moveThenCancel: (full.move(TOP), full.cancelOrder("move")),
			};
			console.log(JSON.stringify(r));
		};"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let snapshot = build_snapshot(&world, &SnapshotOptions { cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 }, always_visible: false, include_terrain: true });
		let res = bot.run_tick(snapshot, "{}".to_string()).unwrap();
		let r: serde_json::Value = serde_json::from_str(res.console.first().map(String::as_str).unwrap_or("{}")).unwrap_or_else(|_| panic!("{:?} {:?}", res.console, res.errors));
		assert_eq!(r["withdrawFarEmpty"], -9, "{r}");
		assert_eq!(r["withdrawCreep"], -7, "{r}");
		assert_eq!(r["withdrawTooMuch"], -9, "距离先于数量: {r}");
		assert_eq!(r["pickupFull"], -8, "{r}");
		assert_eq!(r["say"], 0, "{r}");
		assert_eq!(r["claimGcl"], -15, "{r}");
		assert_eq!(r["cancelMissing"], -5, "{r}");
		assert_eq!(r["moveThenCancel"], 0, "{r}");
		assert!(res.intents.entries.iter().all(|(_, n, _)| n != "move"), "cancelOrder 应撤掉 move 意图");
	});
}

/// 回归: 地形只在首个快照携带, shim 曾在「第一次被访问时」才解析 —— bot 第 2 tick 起才读某房地形时
/// 缓存成全平地且永不更新 (寻路穿墙)。现在收到地形的那一 tick 立即解析全部房间。
#[test]
fn terrain_read_after_first_tick_is_correct() {
	in_tokio(|| {
		let mut rows = vec!["0".repeat(50); 50];
		rows[10] = format!("{}1{}", "0".repeat(10), "0".repeat(39));
		let json = format!(
			r#"{{ "name": "t", "seed": 1, "rooms": [{{ "name": "W5N8", "terrain": {{ "kind": "rows", "rows": {} }},
				"controller": {{ "level": 1, "pos": [40, 40] }}, "structures": [{{ "type": "spawn", "pos": [25, 25], "energy": 300 }}] }}] }}"#,
			serde_json::to_string(&rows).unwrap()
		);
		let world = build_world(&Scenario::from_json(&json).unwrap()).unwrap();
		let bundle = r#"exports.loop = function() {
			if (Game.time < 1) return;
			console.log(JSON.stringify({ wall: Game.map.getRoomTerrain("W5N8").get(10, 10), plain: Game.map.getRoomTerrain("W5N8").get(11, 10) }));
		};"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let mut world = world;
		let mut memory = String::from("{}");
		let mut last = String::new();
		for tick_i in 0..3 {
			let include = bot.take_terrain_pending();
			let snapshot = build_snapshot(&world, &SnapshotOptions { cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 }, always_visible: false, include_terrain: include });
			let r = bot.run_tick(snapshot, memory.clone()).unwrap();
			memory = r.memory.clone();
			if let Some(l) = r.console.first() {
				last = l.clone();
			}
			let mut movement = Movement::default();
			tick::run_tick(&mut world, &mut movement, &r.intents);
			let _ = tick_i;
		}
		let v: serde_json::Value = serde_json::from_str(&last).unwrap_or_else(|_| panic!("无输出: {last}"));
		assert_eq!(v["wall"], 1, "第 2 tick 起才读的地形也要正确: {v}");
		assert_eq!(v["plain"], 0, "{v}");
	});
}
