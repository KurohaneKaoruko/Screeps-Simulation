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
		.map(|p| BodyPart::new(p))
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
		.map(|p| BodyPart::new(p))
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

// ═════════════════════════ 工业 API: 矿物 / 实验室 / 终端 / 观察者 / PowerSpawn / 工厂 ═════════════════════════

const INDUSTRY_SCENE: &str = r#"{
	"name": "industry-js", "seed": 3,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[5, 5]] },
		  "mineral": { "pos": [40, 40], "mineral_type": "H", "amount": 1000 },
		  "controller": { "level": 8, "pos": [25, 25] },
		  "structures": [
			{ "type": "extractor", "pos": [40, 40] },
			{ "type": "lab", "pos": [30, 30], "store": { "energy": 2000, "H": 100 } },
			{ "type": "lab", "pos": [31, 30], "store": { "energy": 2000, "O": 100 } },
			{ "type": "lab", "pos": [32, 31], "store": { "energy": 2000 } },
			{ "type": "terminal", "pos": [20, 20], "store": { "energy": 10000, "H": 5000 } },
			{ "type": "observer", "pos": [21, 20] },
			{ "type": "powerSpawn", "pos": [22, 20], "store": { "energy": 100, "power": 2 } },
			{ "type": "factory", "pos": [23, 20], "store": { "energy": 1000, "U": 1000 } }
		  ],
		  "creeps": [
			{ "name": "Miner", "body": ["work", "work", "work", "work", "work", "carry", "move"], "pos": [41, 41] },
			{ "name": "Hauler", "body": ["carry", "carry", "move"], "pos": [29, 29], "store": { "O": 50 } },
			{ "name": "Booster", "body": [{ "type": "work", "boost": "UO" }, "carry", { "type": "move" }], "pos": [10, 10] }
		  ] },
		{ "name": "W4N8", "terrain": { "kind": "flat" },
		  "controller": { "level": 6, "pos": [25, 25] },
		  "structures": [{ "type": "terminal", "pos": [20, 20] }] },
		{ "name": "W5N9", "terrain": { "kind": "flat" } }
	]
}"#;

/// 工业 API 的返回码与属性 (官方 E/game/structures.js / creeps.js), 意图经引擎结算后下一 tick 可见
#[test]
fn industry_apis_via_js() {
	in_tokio(|| industry_apis_via_js_inner());
}

fn industry_apis_via_js_inner() {
	let scenario = Scenario::from_json(INDUSTRY_SCENE).unwrap();
	let mut world = build_world(&scenario).unwrap();
	let mut movement = Movement::default();
	let bundle = r#"
		exports.loop = function() {
			const room = Game.rooms.W5N8;
			const at = (x, y, t) => room.lookForAt(LOOK_STRUCTURES, x, y).find(s => s.structureType === t);
			const lab1 = at(30, 30, STRUCTURE_LAB), lab2 = at(31, 30, STRUCTURE_LAB), out = at(32, 31, STRUCTURE_LAB);
			const term = room.terminal, obs = at(21, 20, STRUCTURE_OBSERVER), ps = at(22, 20, STRUCTURE_POWER_SPAWN);
			const factory = at(23, 20, STRUCTURE_FACTORY), ext = at(40, 40, STRUCTURE_EXTRACTOR);
			const mineral = room.find(FIND_MINERALS)[0];
			const miner = Game.creeps.Miner, hauler = Game.creeps.Hauler, booster = Game.creeps.Booster;
			const r = { phase: Memory.phase || 0 };
			Memory.phase = r.phase + 1;
			r.harvest = miner.harvest(mineral);
			r.extCooldown = ext ? ext.cooldown : null;
			r.mineral = [mineral.mineralType, mineral.mineralAmount, mineral.density, mineral.ticksToRegeneration === undefined];
			r.gpl = [Game.gpl.level, Game.gpl.progress, Game.gpl.progressTotal];
			r.seeW5N9 = !!Game.rooms.W5N9;
			if (r.phase === 0) {
				r.lab1 = [lab1.mineralType, lab1.mineralAmount, lab1.mineralCapacity, lab1.energy, lab1.energyCapacity, lab1.cooldown];
				r.labCapO = lab1.store.getCapacity(RESOURCE_OXYGEN);
				r.labCapH = lab1.store.getCapacity(RESOURCE_HYDROGEN);
				r.labFreeH = lab1.store.getFreeCapacity(RESOURCE_HYDROGEN);
				r.emptyLabCapO = out.store.getCapacity(RESOURCE_OXYGEN);
				r.transferO = hauler.transfer(lab1, RESOURCE_OXYGEN);
				r.reactNoReagent = lab1.runReaction(lab2, out);
				r.react = out.runReaction(lab1, lab2);
				r.reverseEmpty = out.reverseReaction(lab1, lab2);
				r.boostNothing = lab1.boostCreep(hauler);
				r.boostFar = lab1.boostCreep(booster);
				r.unboostNone = lab1.unboostCreep(hauler);
				r.boosterBody = booster.body.map(p => p.boost || null);
				r.cost = Game.market.calcTransactionCost(1000, "W5N8", "W4N8");
				r.costFar = Game.market.calcTransactionCost(1000, "W5N8", "W5N18");
				r.sendBadRoom = term.send(RESOURCE_HYDROGEN, 1, "nowhere");
				r.sendTooMuch = term.send(RESOURCE_HYDROGEN, 999999, "W4N8");
				r.send = term.send(RESOURCE_HYDROGEN, 1000, "W4N8", "hello");
				r.observeFar = obs.observeRoom("W5N30");
				r.observe = obs.observeRoom("W5N9");
				r.ps = [ps.power, ps.powerCapacity, ps.energy, ps.energyCapacity];
				r.processPower = ps.processPower();
				r.produceBad = factory.produce("nope");
				r.produceLeveled = factory.produce(RESOURCE_COMPOSITE);
				r.produce = factory.produce(RESOURCE_UTRIUM_BAR);
				r.drop = hauler.drop(RESOURCE_OXYGEN, 10);
				r.commodities = COMMODITIES[RESOURCE_UTRIUM_BAR].components.U;
			} else if (r.phase === 1) {
				r.outLab = [out.mineralType, out.mineralAmount, out.cooldown];
				r.termCooldown = term.cooldown;
				r.sendTired = term.send(RESOURCE_HYDROGEN, 100, "W4N8");
				r.outgoing = Game.market.outgoingTransactions.map(t => [t.amount, t.resourceType, t.from, t.to, t.description]);
				r.incoming = Game.market.incomingTransactions.length;
				r.factory = [factory.cooldown, factory.store[RESOURCE_UTRIUM_BAR]];
				const piles = room.find(FIND_DROPPED_RESOURCES);
				r.piles = piles.map(p => [p.resourceType, p.amount, p.energy === undefined]);
				r.look = room.lookForAt(LOOK_RESOURCES, hauler.pos).length + room.lookForAt(LOOK_ENERGY, hauler.pos).length;
				r.reactTired = out.runReaction(lab1, lab2);
			}
			console.log(JSON.stringify(r));
		};
	"#;
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let mut memory = String::from("{}");
	let mut infos = Vec::new();
	for i in 0..3 {
		if i == 2 {
			// 拆掉 extractor: 第三 tick harvest(mineral) 应 ERR_NOT_FOUND
			let room = world.rooms.get_mut("W5N8").unwrap();
			let ext = room.objects.values().find(|o| o.obj_type == "extractor").unwrap().id;
			room.objects.remove(&ext);
		}
		let snapshot = build_snapshot(
			&world,
			&SnapshotOptions {
				cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
				always_visible: false,
				include_terrain: true,
			},
		);
		let result = bot.run_tick(snapshot, memory.clone()).unwrap();
		assert!(result.errors.is_empty(), "bundle 不应报错: {:?}", result.errors);
		memory = result.memory.clone();
		infos.push(serde_json::from_str::<serde_json::Value>(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap());
		tick::run_tick(&mut world, &mut movement, &result.intents);
	}
	let t0 = &infos[0];
	assert_eq!(t0["harvest"], 0, "{t0}");
	assert_eq!(t0["extCooldown"], 0, "{t0}");
	assert_eq!(t0["mineral"], serde_json::json!(["H", 1000, 2, true]), "{t0}");
	assert_eq!(t0["gpl"], serde_json::json!([0, 0, 1000]), "{t0}");
	assert_eq!(t0["seeW5N9"], false, "{t0}");
	assert_eq!(t0["lab1"], serde_json::json!(["H", 100, 3000, 2000, 2000, 0]), "{t0}");
	assert_eq!(t0["labCapO"], serde_json::Value::Null, "持有 H 的 lab 对 O 容量为 0 → null: {t0}");
	assert_eq!(t0["labCapH"], 3000, "{t0}");
	assert_eq!(t0["labFreeH"], 2900, "{t0}");
	assert_eq!(t0["emptyLabCapO"], 3000, "{t0}");
	assert_eq!(t0["transferO"], -7, "官方: capacityForResource 为 0 → ERR_INVALID_TARGET: {t0}");
	assert_eq!(t0["reactNoReagent"], -6, "{t0}");
	assert_eq!(t0["react"], 0, "{t0}");
	assert_eq!(t0["reverseEmpty"], -6, "{t0}");
	assert_eq!(t0["boostNothing"], -5, "H 不能强化任何部件 → ERR_NOT_FOUND: {t0}");
	assert_eq!(t0["boostFar"], -9, "{t0}");
	assert_eq!(t0["unboostNone"], -5, "{t0}");
	assert_eq!(t0["boosterBody"], serde_json::json!(["UO", null, null]), "{t0}");
	assert_eq!(t0["cost"], 33, "{t0}");
	assert_eq!(t0["costFar"], 284, "ceil(1000 × (1 − e^(−10/30))): {t0}");
	assert_eq!(t0["sendBadRoom"], -10, "{t0}");
	assert_eq!(t0["sendTooMuch"], -6, "{t0}");
	assert_eq!(t0["send"], 0, "{t0}");
	assert_eq!(t0["observeFar"], -9, "{t0}");
	assert_eq!(t0["observe"], 0, "{t0}");
	assert_eq!(t0["ps"], serde_json::json!([2, 100, 100, 5000]), "{t0}");
	assert_eq!(t0["processPower"], 0, "{t0}");
	assert_eq!(t0["produceBad"], -10, "{t0}");
	assert_eq!(t0["produceLeveled"], -7, "1 级商品 vs 无等级工厂 → ERR_INVALID_TARGET: {t0}");
	assert_eq!(t0["produce"], 0, "{t0}");
	assert_eq!(t0["drop"], 0, "{t0}");
	assert_eq!(t0["commodities"], 500, "{t0}");

	let t1 = &infos[1];
	assert_eq!(t1["harvest"], -11, "extractor 冷却中 → ERR_TIRED: {t1}");
	assert_eq!(t1["extCooldown"], 5, "{t1}");
	assert_eq!(t1["mineral"], serde_json::json!(["H", 995, 2, true]), "{t1}");
	assert_eq!(t1["gpl"], serde_json::json!([0, 1, 1000]), "{t1}");
	assert_eq!(t1["seeW5N9"], true, "observeRoom 的下一 tick 可见: {t1}");
	assert_eq!(t1["outLab"], serde_json::json!(["OH", 5, 19]), "{t1}");
	assert_eq!(t1["termCooldown"], 9, "{t1}");
	assert_eq!(t1["sendTired"], -11, "{t1}");
	assert_eq!(t1["outgoing"], serde_json::json!([[1000, "H", "W5N8", "W4N8", "hello"]]), "{t1}");
	assert_eq!(t1["incoming"], 1, "单玩家: 发往自己的终端同时计入 incoming: {t1}");
	assert_eq!(t1["factory"], serde_json::json!([19, 100]), "{t1}");
	assert_eq!(t1["piles"], serde_json::json!([["O", 9, true]]), "{t1}");
	assert_eq!(t1["look"], 2, "LOOK_RESOURCES 与 LOOK_ENERGY 都返回非能量堆: {t1}");
	assert_eq!(t1["reactTired"], -11, "{t1}");

	let t2 = &infos[2];
	assert_eq!(t2["harvest"], -5, "没有 extractor → ERR_NOT_FOUND: {t2}");
	assert_eq!(t2["seeW5N9"], false, "观察只持续一 tick: {t2}");
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
