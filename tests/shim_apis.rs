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

// ═════════════════════════ 官方 API 保真回归 (审计条目 1-17) ═════════════════════════

/// 保真测试场景: W5N8 (RCL3, 有视野) + W5N7 (南侧, 无视野) + W6N8 (西侧, 中立控制器, 靠侦察兵可见)
const FIDELITY_ROOMS: &str = r#"{
	"name": "fidelity",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "mineral": { "pos": [40, 40], "mineral_type": "H" },
		  "controller": { "level": 3, "pos": [28, 27] },
		  "structures": [
			{ "type": "spawn", "pos": [25, 25], "energy": 300 },
			{ "type": "road", "pos": [10, 10] },
			{ "type": "rampart", "pos": [12, 10] },
			{ "type": "rampart", "pos": [14, 10], "enemy": true }
		  ],
		  "creeps": [
			{ "name": "A", "body": ["move", "work", "carry"], "pos": [20, 20] },
			{ "name": "B", "body": ["move"], "pos": [22, 20] },
			{ "name": "Foe", "body": ["move"], "pos": [30, 20], "enemy": true }
		  ] },
		{ "name": "W5N7", "terrain": { "kind": "flat" } },
		{ "name": "W6N8", "terrain": { "kind": "flat" },
		  "controller": { "level": 0, "pos": [25, 25], "mine": false },
		  "creeps": [{ "name": "Scout", "body": ["move"], "pos": [10, 10] }] }
	]
}"#;

fn fidelity_world() -> screeps_sim::engine::world::World {
	build_world(&Scenario::from_json(FIDELITY_ROOMS).unwrap()).unwrap()
}

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

/// 跑一 tick bundle, 返回 (第一行 console 解析的 JSON, 意图, 全部 console, errors)
fn run_json(
	bundle: &str,
	world: &screeps_sim::engine::world::World,
) -> (serde_json::Value, Vec<(String, String, serde_json::Value)>, Vec<String>) {
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let result = bot.run_tick(snapshot_of(world), "{}".to_string()).unwrap();
	assert!(result.errors.is_empty(), "bundle 不应抛错: {:?}", result.errors);
	let info: serde_json::Value =
		serde_json::from_str(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
	(info, result.intents.entries, result.console)
}

/// 多 tick: 同一 isolate 跑 bundle → 意图交引擎结算 → 下一 tick 快照, 返回每 tick 第一行 console JSON
fn run_ticks(bundle: &str, world: &mut screeps_sim::engine::world::World, n: usize) -> Vec<serde_json::Value> {
	let mut bot = BotRuntime::new(bundle, 1).unwrap();
	let mut memory = String::from("{}");
	let mut movement = Movement::default();
	let mut out = Vec::new();
	for _ in 0..n {
		let result = bot.run_tick(snapshot_of(world), memory.clone()).unwrap();
		assert!(result.errors.is_empty(), "bundle 不应抛错: {:?}", result.errors);
		memory = result.memory.clone();
		out.push(
			serde_json::from_str::<serde_json::Value>(result.console.first().map(String::as_str).unwrap_or("{}"))
				.unwrap(),
		);
		tick::run_tick(world, &mut movement, &result.intents);
	}
	out
}

/// 审计 #1/#14/#15: fetchXYArguments 重载、RoomPosition 构造校验、无视野时官方抛错
#[test]
fn roompos_fetch_xy_and_no_vision_semantics() {
	in_tokio(|| {
		let world = fidelity_world();
		let bundle = r#"
			exports.loop = function() {
				const a = Game.creeps.A, b = Game.creeps.B, r = a.room;
				const out = {};
				out.ccsPos = r.createConstructionSite(new RoomPosition(21, 21, "W5N8"), STRUCTURE_ROAD);
				out.ccsObj = r.createConstructionSite(b, STRUCTURE_ROAD);
				out.ccsPlain = r.createConstructionSite({ x: 23, y: 23 }, STRUCTURE_ROAD);
				out.nearXY = a.pos.isNearTo(21, 21);
				out.nearXYFar = a.pos.isNearTo(22, 22);
				out.nearObj = a.pos.isNearTo(b);
				out.inRangeXY = a.pos.inRangeTo(22, 20, 2);
				out.inRangeObj = a.pos.inRangeTo(b, 1);
				out.rangeObj = a.pos.getRangeTo(b);
				out.rangeXY = a.pos.getRangeTo(22, 23);
				out.rangeOtherRoom = a.pos.getRangeTo(new RoomPosition(20, 20, "W5N7"));
				out.lookObj = r.lookForAt(LOOK_CREEPS, b).length;
				out.lookAtObj = r.lookAt(b).map((i) => i.type);
				const err = (f) => { try { f(); return "no-throw"; } catch (e) { return e.message; } };
				out.ctorRange = err(() => new RoomPosition(50, 1, "W5N8"));
				out.ctorNaN = err(() => new RoomPosition(NaN, 1, "W5N8"));
				out.ctorRoom = err(() => new RoomPosition(1, 1, "nowhere"));
				const c = new RoomPosition("7", 8.9, "W5N8");
				out.coerced = [c.x, c.y, typeof c.x];
				const far = new RoomPosition(10, 10, "W5N7");
				out.noVision = {
					findInRange: err(() => far.findInRange(FIND_CREEPS, 3)),
					findClosestByRange: err(() => far.findClosestByRange(FIND_CREEPS)),
					findClosestByPath: err(() => far.findClosestByPath(FIND_CREEPS)),
					findPathTo: err(() => far.findPathTo(12, 12)),
					look: err(() => far.look()),
					lookFor: err(() => far.lookFor(LOOK_CREEPS)),
					createFlag: err(() => far.createFlag("X")),
					createConstructionSite: err(() => far.createConstructionSite(STRUCTURE_ROAD)),
				};
				out.terrainNoVision = far.lookFor(LOOK_TERRAIN);
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, entries, _) = run_json(bundle, &world);
		assert_eq!(info["ccsPos"], 0, "createConstructionSite(RoomPosition, type) 应 OK: {info}");
		assert_eq!(info["ccsObj"], 0, "createConstructionSite(RoomObject, type) 取 .pos 并 OK: {info}");
		assert_eq!(info["ccsPlain"], -10, "普通 {{x,y}} 对象官方不接受 → ERR_INVALID_ARGS: {info}");
		let sites: Vec<(u64, u64)> = entries
			.iter()
			.filter(|(_, n, _)| n == "createConstructionSite")
			.map(|(_, _, d)| (d["x"].as_u64().unwrap(), d["y"].as_u64().unwrap()))
			.collect();
		assert_eq!(sites, vec![(21, 21), (22, 20)], "对象形式的坐标应来自 pos: {sites:?}");
		assert_eq!(info["nearXY"], true, "isNearTo(x, y) 此前恒 false: {info}");
		assert_eq!(info["nearXYFar"], false);
		assert_eq!(info["nearObj"], false, "B 距离 2: {info}");
		assert_eq!(info["inRangeXY"], true);
		assert_eq!(info["inRangeObj"], false);
		assert_eq!(info["rangeObj"], 2);
		assert_eq!(info["rangeXY"], 3);
		assert_eq!(info["rangeOtherRoom"], serde_json::Value::Null, "跨房 Infinity (JSON 为 null): {info}");
		assert_eq!(info["lookObj"], 1, "lookForAt(type, RoomObject): {info}");
		assert_eq!(info["lookAtObj"], serde_json::json!(["creep", "terrain"]), "{info}");
		for k in ["ctorRange", "ctorNaN", "ctorRoom"] {
			assert_eq!(info[k], "Invalid arguments in RoomPosition constructor", "{k}: {info}");
		}
		assert_eq!(info["coerced"], serde_json::json!([7, 8, "number"]), "坐标按 packed 存储截断为整数: {info}");
		for (k, v) in info["noVision"].as_object().unwrap() {
			assert_eq!(v, "Could not access room W5N7", "{k} 无视野应抛官方错误: {info}");
		}
		assert_eq!(info["terrainNoVision"], serde_json::json!(["plain"]), "LOOK_TERRAIN 不需要视野: {info}");
	});
}

/// 审计 #2: Room.createConstructionSite 的校验顺序与 checkConstructionSite / checkControllerAvailability
#[test]
fn construction_site_validation_matches_official() {
	in_tokio(|| {
		let mut world = fidelity_world();
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8, w = Game.rooms.W6N8;
				const out = {};
				if (Game.time > 1) {
					out.siteNames = Object.values(Game.constructionSites)
						.filter((s) => s.structureType === STRUCTURE_SPAWN).map((s) => s.name).sort();
					console.log(JSON.stringify(out));
					return;
				}
				out.roadOnRoad = r.createConstructionSite(10, 10, STRUCTURE_ROAD);
				out.rampartOnRampart = r.createConstructionSite(12, 10, STRUCTURE_RAMPART);
				out.containerOnSpawn = r.createConstructionSite(25, 25, STRUCTURE_CONTAINER);
				out.rampartOnSpawn = r.createConstructionSite(25, 25, STRUCTURE_RAMPART);
				out.roadOnRampart = r.createConstructionSite(12, 10, STRUCTURE_ROAD);
				out.siteOnSite = r.createConstructionSite(12, 10, STRUCTURE_WALL);
				out.edge0 = r.createConstructionSite(0, 20, STRUCTURE_ROAD);
				out.edge49 = r.createConstructionSite(20, 49, STRUCTURE_CONTAINER);
				out.border1Ext = r.createConstructionSite(1, 20, STRUCTURE_EXTENSION);
				out.border1Road = r.createConstructionSite(1, 20, STRUCTURE_ROAD);
				out.extractorNoMineral = r.createConstructionSite(33, 33, STRUCTURE_EXTRACTOR);
				out.spawnDupName = r.createConstructionSite(30, 30, STRUCTURE_SPAWN, "Spawn1");
				out.spawnLongName = r.createConstructionSite(30, 30, STRUCTURE_SPAWN, "x".repeat(101));
				out.spawnRcl = r.createConstructionSite(30, 30, STRUCTURE_SPAWN);
				out.badType = r.createConstructionSite(30, 30, "nope");
				// 中立控制器房间按 RCL 0: extension 0 个, container 5 个, spawn 0 个 (但名字校验在前)
				out.neutralExt = w.createConstructionSite(20, 20, STRUCTURE_EXTENSION);
				out.neutralContainer = w.createConstructionSite(20, 20, STRUCTURE_CONTAINER);
				out.neutralSpawn = w.createConstructionSite(22, 22, STRUCTURE_SPAWN);
				out.neutralCtrlMy = w.controller.my === undefined && w.controller.owner === undefined;
				console.log(JSON.stringify(out));
			};
		"#;
		let infos = run_ticks(bundle, &mut world, 2);
		let info = &infos[0];
		assert_eq!(info["roadOnRoad"], -7, "路上不能再建路: {info}");
		assert_eq!(info["rampartOnRampart"], -7, "rampart 上不能再建 rampart: {info}");
		assert_eq!(info["containerOnSpawn"], -7, "容器不能与 spawn 同格: {info}");
		assert_eq!(info["rampartOnSpawn"], 0, "rampart 可以盖在 spawn 上: {info}");
		assert_eq!(info["roadOnRampart"], 0, "路可以修在 rampart 下: {info}");
		assert_eq!(info["siteOnSite"], 0, "本 tick 新建的工地不进快照 (官方同样只看 tick 开始的对象): {info}");
		assert_eq!(info["edge0"], -7, "x=0 的出口格拒绝: {info}");
		assert_eq!(info["edge49"], -7, "y=49 的出口格拒绝: {info}");
		assert_eq!(info["border1Ext"], -7, "1 行列外侧不是墙 → 拒绝: {info}");
		assert_eq!(info["border1Road"], 0, "路不受 1/48 规则限制: {info}");
		assert_eq!(info["extractorNoMineral"], -14, "RCL3 extractor 上限为 0 (可用性先于位置检查): {info}");
		assert_eq!(info["spawnDupName"], -10, "已有 spawn 同名 → ERR_INVALID_ARGS: {info}");
		assert_eq!(info["spawnLongName"], -10, "名字超过 100: {info}");
		assert_eq!(info["spawnRcl"], -14, "RCL3 只能有 1 个 spawn: {info}");
		assert_eq!(info["badType"], -10);
		assert_eq!(info["neutralExt"], -14, "无自有控制器按 RCL 0: {info}");
		assert_eq!(info["neutralContainer"], 0, "{info}");
		assert_eq!(info["neutralSpawn"], -14, "{info}");
		assert_eq!(info["neutralCtrlMy"], true, "中立控制器 my/owner 为 undefined: {info}");
		// 引擎侧: siteOnSite (墙叠在同 tick 新建的路工地上) 被处理器拒绝; 路/rampart 工地落地
		let sites: Vec<(String, u8, u8)> = world.rooms["W5N8"]
			.objects
			.values()
			.filter(|o| o.obj_type == "constructionSite")
			.map(|o| (o.structure_type.clone().unwrap(), o.x, o.y))
			.collect();
		assert!(sites.contains(&("rampart".into(), 25, 25)), "{sites:?}");
		assert!(sites.contains(&("road".into(), 12, 10)), "{sites:?}");
		assert!(!sites.iter().any(|s| s.0 == "constructedWall"), "同格已有工地时处理器拒绝: {sites:?}");
		assert!(sites.contains(&("road".into(), 1, 20)), "{sites:?}");
	});
}

/// 审计 #2/#11: spawn 工地自动命名 / 名字随工地下发; 工地不提供视野但仍在 Game.constructionSites;
/// MAX_CONSTRUCTION_SITES 在位置校验之后检查
#[test]
fn spawn_site_names_and_sites_without_vision() {
	in_tokio(|| {
		let mut world = fidelity_world();
		// W5N7 (无视野) 放 99 个自己的工地 → 只剩 1 个名额
		{
			use screeps_sim::engine::objects::construction_site;
			let mut next = world.next_id;
			let room = world.rooms.get_mut("W5N7").unwrap();
			for i in 0..99u8 {
				let id = screeps_sim::engine::world::alloc_id(&mut next);
				room.objects.insert(id, construction_site(id, "road", 2 + i % 45, 2 + i / 45, 300));
			}
			world.next_id = next;
		}
		assert!(!world.rooms["W5N7"].visible(), "只有工地的房间不可见");
		// RCL 7 允许第二个 spawn
		world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().level = 7;
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8;
				const out = {};
				const sites = Object.values(Game.constructionSites);
				out.total = sites.length;
				const far = sites.find((s) => s.pos.roomName === "W5N7");
				out.farRoomUndefined = far ? far.room === undefined : null;
				out.farById = far ? Game.getObjectById(far.id) === far : null;
				out.w5n7Visible = !!Game.rooms.W5N7;
				out.spawnSites = sites.filter((s) => s.structureType === STRUCTURE_SPAWN).map((s) => s.name);
				if (!global.__firstTickDone) {
					global.__firstTickDone = true;
					out.auto = r.createConstructionSite(30, 30, STRUCTURE_SPAWN);
					// 满额后: 位置非法仍先报 ERR_INVALID_TARGET, 合法才报 ERR_FULL
					out.fullInvalid = r.createConstructionSite(10, 10, STRUCTURE_ROAD);
					out.fullValid = r.createConstructionSite(31, 31, STRUCTURE_ROAD);
				}
				console.log(JSON.stringify(out));
			};
		"#;
		let infos = run_ticks(bundle, &mut world, 2);
		let t1 = &infos[0];
		assert_eq!(t1["total"], 99, "无视野房间的工地也在 Game.constructionSites: {t1}");
		assert_eq!(t1["farRoomUndefined"], true, "{t1}");
		assert_eq!(t1["farById"], true, "{t1}");
		assert_eq!(t1["w5n7Visible"], false, "工地不提供视野: {t1}");
		assert_eq!(t1["auto"], 0, "{t1}");
		assert_eq!(t1["fullInvalid"], -7, "{t1}");
		assert_eq!(t1["fullValid"], -8, "99 + 本 tick 1 = 100 → ERR_FULL: {t1}");
		let t2 = &infos[1];
		assert_eq!(t2["total"], 100, "{t2}");
		assert_eq!(t2["spawnSites"], serde_json::json!(["Spawn2"]), "未给名字时按官方自动命名 Spawn2: {t2}");
		let site = world.rooms["W5N8"]
			.objects
			.values()
			.find(|o| o.obj_type == "constructionSite" && o.structure_type.as_deref() == Some("spawn"))
			.unwrap();
		assert_eq!(site.name.as_deref(), Some("Spawn2"), "引擎工地记录 spawn 名");
	});
}

/// 审计 #3: findInRange / findClosestByRange / findClosestByPath 的数组形式与 opts.filter
#[test]
fn find_helpers_accept_arrays_and_filters() {
	in_tokio(|| {
		let world = fidelity_world();
		let bundle = r#"
			exports.loop = function() {
				const a = Game.creeps.A;
				const all = a.room.find(FIND_CREEPS);
				const out = {};
				out.inRangeArr = a.pos.findInRange(all, 2).map((c) => c.name).sort();
				out.inRangeFilter = a.pos.findInRange(all, 2, { filter: (c) => c.name === "B" }).map((c) => c.name);
				out.inRangePositions = a.pos.findInRange([new RoomPosition(21, 21, "W5N8"), new RoomPosition(30, 30, "W5N8")], 1).length;
				out.closestRangeFilter = a.pos.findClosestByRange(all, { filter: (c) => c.name !== "A" }).name;
				out.closestRangeObjFilter = a.pos.findClosestByRange(all, { filter: { name: "Foe" } }).name;
				out.closestPathFilter = a.pos.findClosestByPath(all, { filter: { name: "Foe" } }).name;
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(info["inRangeArr"], serde_json::json!(["A", "B"]), "数组形式此前恒为 []: {info}");
		assert_eq!(info["inRangeFilter"], serde_json::json!(["B"]), "{info}");
		assert_eq!(info["inRangePositions"], 1, "{info}");
		assert_eq!(info["closestRangeFilter"], "B", "数组形式应用 filter: {info}");
		assert_eq!(info["closestRangeObjFilter"], "Foe", "{info}");
		assert_eq!(info["closestPathFilter"], "Foe", "{info}");
	});
}

/// 审计 #4: FIND_* 语义 (孵化中的 creep 不计入, 出口顺序 TOP/BOTTOM/RIGHT/LEFT, 敌方建筑/工地)
#[test]
fn find_constants_match_official() {
	in_tokio(|| {
		let mut world = fidelity_world();
		{
			use screeps_sim::engine::objects::{construction_site, creep_obj, BodyPart};
			let mut next = world.next_id;
			let room = world.rooms.get_mut("W5N8").unwrap();
			let id = screeps_sim::engine::world::alloc_id(&mut next);
			let mut baby = creep_obj(id, "Baby", 25, 26, vec![BodyPart { part_type: "move".into(), hits: 100 }]);
			baby.spawning = true;
			room.objects.insert(id, baby);
			let id = screeps_sim::engine::world::alloc_id(&mut next);
			let mut hostile = construction_site(id, "road", 33, 33, 300);
			hostile.mine = false;
			hostile.enemy = true;
			room.objects.insert(id, hostile);
			let id = screeps_sim::engine::world::alloc_id(&mut next);
			room.objects.insert(id, construction_site(id, "road", 34, 34, 300));
			world.next_id = next;
		}
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8;
				const names = (l) => l.map((c) => c.name).sort();
				const out = {};
				out.gameCreeps = Object.keys(Game.creeps).sort();
				out.creeps = names(r.find(FIND_CREEPS));
				out.myCreeps = names(r.find(FIND_MY_CREEPS));
				out.hostileCreeps = names(r.find(FIND_HOSTILE_CREEPS));
				out.sites = r.find(FIND_CONSTRUCTION_SITES).length;
				out.mySites = r.find(FIND_MY_CONSTRUCTION_SITES).length;
				out.hostileSites = r.find(FIND_HOSTILE_CONSTRUCTION_SITES).length;
				out.hostileStructures = r.find(FIND_HOSTILE_STRUCTURES).map((s) => s.structureType);
				out.myStructures = r.find(FIND_MY_STRUCTURES).map((s) => s.structureType).sort();
				out.empty = [FIND_RUINS, FIND_NUKES, FIND_DEPOSITS, FIND_POWER_CREEPS, FIND_MY_POWER_CREEPS, FIND_HOSTILE_POWER_CREEPS]
					.map((t) => r.find(t).length);
				const exits = r.find(FIND_EXIT);
				out.exitCount = exits.length;
				out.exitOrder = [exits[0], exits[50], exits[100], exits[150]].map((p) => [p.x, p.y]);
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(info["gameCreeps"], serde_json::json!(["A", "B", "Baby", "Scout"]), "Game.creeps 含孵化中: {info}");
		assert_eq!(info["creeps"], serde_json::json!(["A", "B", "Foe"]), "FIND_CREEPS 排除孵化中: {info}");
		assert_eq!(info["myCreeps"], serde_json::json!(["A", "B"]), "{info}");
		assert_eq!(info["hostileCreeps"], serde_json::json!(["Foe"]), "{info}");
		assert_eq!(info["sites"], 2, "FIND_CONSTRUCTION_SITES 含全部工地: {info}");
		assert_eq!(info["mySites"], 1, "{info}");
		assert_eq!(info["hostileSites"], 1, "{info}");
		assert_eq!(info["hostileStructures"], serde_json::json!(["rampart"]), "{info}");
		assert_eq!(
			info["myStructures"],
			serde_json::json!(["controller", "rampart", "spawn"]),
			"FIND_MY_STRUCTURES 只含 my===true (路/敌方 rampart 不在): {info}"
		);
		assert_eq!(info["empty"], serde_json::json!([0, 0, 0, 0, 0, 0]), "{info}");
		assert_eq!(info["exitCount"], 200, "四条边各 50 (角格在两条边上各算一次): {info}");
		assert_eq!(
			info["exitOrder"],
			serde_json::json!([[0, 0], [0, 49], [49, 0], [0, 0]]),
			"官方顺序 TOP, BOTTOM, RIGHT, LEFT: {info}"
		);
	});
}

/// 审计 #5/#6: findClosestByPath 官方算法 (不可达 null, 多个相邻取最后一个, maxRooms 1) 与
/// costCallback / portal / 工地障碍 / serialize / Room.findPath 房间检查
#[test]
fn closest_by_path_and_cost_callback() {
	in_tokio(|| {
		let mut world = fidelity_world();
		{
			let room = world.rooms.get_mut("W5N8").unwrap();
			// (40,10) 四周一圈墙: 不可达
			for dx in -1i32..=1 {
				for dy in -1i32..=1 {
					if dx != 0 || dy != 0 {
						room.terrain.set((40 + dx) as usize, (10 + dy) as usize, 1);
					}
				}
			}
			use screeps_sim::engine::objects::{construction_site, structure};
			let mut next = world.next_id;
			let id = screeps_sim::engine::world::alloc_id(&mut next);
			room.objects.insert(id, structure(id, "portal", 30, 35, false));
			let id = screeps_sim::engine::world::alloc_id(&mut next);
			room.objects.insert(id, construction_site(id, "spawn", 30, 40, 15000));
			// 工地两侧是墙: 只有穿过工地格才是直线
			room.terrain.set(30, 39, 1);
			room.terrain.set(30, 41, 1);
			room.refresh_terrain_cache();
			world.next_id = next;
		}
		let bundle = r#"
			exports.loop = function() {
				const a = Game.creeps.A, r = a.room;
				const out = {};
				const p = (x, y, n) => new RoomPosition(x, y, n || "W5N8");
				out.unreachable = a.pos.findClosestByPath([p(40, 10)]);
				out.rangeStillFinds = !!a.pos.findClosestByRange([p(40, 10)]);
				out.tie = String(a.pos.findClosestByPath([p(21, 21), p(21, 20), p(18, 18)]));
				out.otherRoom = a.pos.findClosestByPath([p(20, 1, "W5N7")]);
				out.blockedByCallback = a.pos.findClosestByPath([p(30, 20)], {
					costCallback: (name, m) => { for (let y = 0; y < 50; y++) m.set(25, y, 255); },
				});
				let sawClone = null;
				const cached = PathFinder.CostMatrix;
				const path = a.pos.findPathTo(26, 20, {
					costCallback: (name, m) => { sawClone = m instanceof cached; for (let y = 0; y < 50; y++) if (y !== 5) m.set(23, y, 255); },
				});
				out.detour = path.some((s) => s.x === 23 && s.y === 5);
				out.sawCostMatrix = sawClone;
				out.noCallbackStraight = a.pos.findPathTo(26, 20).length;
				out.portalStep = JSON.stringify(p(29, 35).findPathTo(31, 35)[0]);
				out.siteBlocks = JSON.stringify(p(29, 40).findPathTo(31, 40)[0]);
				out.siteIgnored = JSON.stringify(p(29, 40).findPathTo(31, 40, { ignoreDestructibleStructures: true })[0]);
				out.serializeSame = r.findPath(a.pos, a.pos, { serialize: true });
				out.otherRoomFrom = r.findPath(p(1, 1, "W5N7"), a.pos);
				out.otherRoomFromSer = r.findPath(p(1, 1, "W5N7"), a.pos, { serialize: true });
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(info["unreachable"], serde_json::Value::Null, "不可达目标返回 null: {info}");
		assert_eq!(info["rangeStillFinds"], true);
		assert_eq!(info["tie"], "[room W5N8 pos 21,20]", "多个相邻目标取列表中最后一个: {info}");
		assert_eq!(info["otherRoom"], serde_json::Value::Null, "maxRooms 1: 邻房目标找不到: {info}");
		assert_eq!(info["blockedByCallback"], serde_json::Value::Null, "costCallback 生效: {info}");
		assert_eq!(info["detour"], true, "findPathTo 的 costCallback 生效 (绕行缺口): {info}");
		assert_eq!(info["sawCostMatrix"], true, "{info}");
		assert_eq!(info["noCallbackStraight"], 6, "回调改的是克隆, 缓存矩阵不受影响: {info}");
		let portal: serde_json::Value = serde_json::from_str(info["portalStep"].as_str().unwrap()).unwrap();
		assert_ne!((portal["x"].as_u64(), portal["y"].as_u64()), (Some(30), Some(35)), "portal 是障碍: {info}");
		let blocked: serde_json::Value = serde_json::from_str(info["siteBlocks"].as_str().unwrap()).unwrap();
		assert_ne!((blocked["x"].as_u64(), blocked["y"].as_u64()), (Some(30), Some(40)), "自己的障碍类工地挡路: {info}");
		let ignored: serde_json::Value = serde_json::from_str(info["siteIgnored"].as_str().unwrap()).unwrap();
		assert_eq!((ignored["x"].as_u64(), ignored["y"].as_u64()), (Some(30), Some(40)), "ignoreDestructibleStructures 时工地不挡路: {info}");
		assert_eq!(info["serializeSame"], "", "起终点相同 + serialize → '': {info}");
		assert_eq!(info["otherRoomFrom"], serde_json::json!([]), "{info}");
		assert_eq!(info["otherRoomFromSer"], "", "{info}");
	});
}

/// 审计 #7/#8: PathFinder flee 语义、边界格不能横走、选项 clamp、CostMatrix._bits、无全局 CostMatrix
#[test]
fn pathfinder_native_semantics() {
	in_tokio(|| {
		let mut world = fidelity_world();
		{
			// W5N8 底边 (y=48) 9..13 一排墙: (10,49) → (12,49) 只能经 W5N7 绕行
			let room = world.rooms.get_mut("W5N8").unwrap();
			for x in 9..=13 {
				room.terrain.set(x, 48, 1);
			}
			room.refresh_terrain_cache();
		}
		let bundle = r#"
			exports.loop = function() {
				const out = {};
				const p = (x, y, n) => new RoomPosition(x, y, n || "W5N8");
				const fl = PathFinder.search(p(20, 20), { pos: p(22, 22), range: 3 }, { flee: true });
				out.flee = { len: fl.path.length, incomplete: fl.incomplete, last: String(fl.path[fl.path.length - 1]) };
				const already = PathFinder.search(p(20, 20), { pos: p(25, 25), range: 3 }, { flee: true });
				out.fleeNoop = [already.path.length, already.incomplete];
				const edge = PathFinder.search(p(10, 49), p(12, 49));
				out.edge = edge.path.map((q) => q.roomName + ":" + q.x + "," + q.y);
				out.edgeIncomplete = edge.incomplete;
				out.costClamp = PathFinder.search(p(20, 20), p(23, 20), { plainCost: 300 }).cost;
				out.costFloor = PathFinder.search(p(20, 20), p(23, 20), { plainCost: 2.7 }).cost;
				out.costZero = PathFinder.search(p(20, 20), p(23, 20), { plainCost: 0 }).cost;
				out.emptyGoal = PathFinder.search(p(20, 20), []);
				const m = new PathFinder.CostMatrix();
				m.set(1, 2, 300);
				out.bits = [typeof m._bits, m.__bits === undefined, m._bits[1 * 50 + 2]];
				out.globalCostMatrix = typeof CostMatrix;
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(info["flee"]["len"], 1, "flee: 一步离开 range 3 即停: {info}");
		assert_eq!(info["flee"]["incomplete"], false, "{info}");
		assert_eq!(info["fleeNoop"], serde_json::json!([0, false]), "起点已在 range 外: 空路径且完整: {info}");
		assert_eq!(
			info["edge"],
			serde_json::json!(["W5N7:10,0", "W5N7:11,1", "W5N7:12,0", "W5N8:12,49"]),
			"边界格只能笔直跨房或退回房内, 不能沿出口格横走: {info}"
		);
		assert_eq!(info["edgeIncomplete"], false);
		assert_eq!(info["costClamp"], 3 * 254, "plainCost clamp 到 254: {info}");
		assert_eq!(info["costFloor"], 3 * 2, "plainCost 取整: {info}");
		assert_eq!(info["costZero"], 3, "plainCost 0 → 默认 1: {info}");
		assert_eq!(info["emptyGoal"], serde_json::json!({ "path": [], "ops": 0 }), "{info}");
		assert_eq!(info["bits"], serde_json::json!(["object", true, 255]), "{info}");
		assert_eq!(info["globalCostMatrix"], "undefined", "官方只有 PathFinder.CostMatrix: {info}");
	});
}

/// 审计 #9: 旗经引擎持久化 (createFlag / setColor / setPosition / remove), 参数前移, COLORS_ALL, 名字长度
#[test]
fn flags_persist_through_engine() {
	in_tokio(|| {
		let mut world = fidelity_world();
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8;
				const out = { t: Game.time };
				const step = Memory.step || 0;
				Memory.step = step + 1;
				out.step = step;
				if (step === 0) {
					out.colorsAll = COLORS_ALL.length;
					out.f1 = r.createFlag(10, 12, "F1");
					out.sameTick = Game.flags.F1 instanceof Flag && Game.flags.F1.color === COLOR_WHITE;
					out.f2 = r.createFlag(new RoomPosition(11, 11, "W5N8"), "F2", COLOR_RED, COLOR_BLUE);
					out.auto = r.createFlag(5, 5);
					out.dup = r.createFlag(6, 6, "F1");
					out.badColor = r.createFlag(6, 6, "F3", 11);
					out.longName = r.createFlag(6, 6, "x".repeat(101));
					out.badPos = r.createFlag(50, 6, "F4");
					out.posForm = new RoomPosition(7, 7, "W5N8").createFlag("F5", COLOR_GREEN);
				} else if (step === 1) {
					out.names = Object.keys(Game.flags).sort();
					out.f2 = [Game.flags.F2.color, Game.flags.F2.secondaryColor];
					out.f5 = [Game.flags.F5.color, Game.flags.F5.secondaryColor];
					out.look = r.lookForAt(LOOK_FLAGS, 11, 11).map((f) => f.name);
					out.find = r.find(FIND_FLAGS).length;
					out.byId = Game.getObjectById("flag_F1") === Game.flags.F1;
					out.setColor = Game.flags.F1.setColor(COLOR_GREEN);
					out.setColorBad = Game.flags.F1.setColor(0);
					out.move = Game.flags.F2.setPosition(new RoomPosition(5, 5, "W5N7"));
					out.remove = Game.flags.Flag1.remove();
				} else {
					out.names = Object.keys(Game.flags).sort();
					out.f1 = [Game.flags.F1.color, Game.flags.F1.secondaryColor];
					out.f2Room = Game.flags.F2.pos.roomName;
					out.f2RoomUndefined = Game.flags.F2.room === undefined;
					out.find = r.find(FIND_FLAGS).length;
				}
				console.log(JSON.stringify(out));
			};
		"#;
		let infos = run_ticks(bundle, &mut world, 3);
		let t = &infos[0];
		assert_eq!(t["colorsAll"], 10, "{t}");
		assert_eq!(t["f1"], "F1", "{t}");
		assert_eq!(t["sameTick"], true, "同 tick 就进 Game.flags: {t}");
		assert_eq!(t["f2"], "F2", "对象形式参数前移: {t}");
		assert_eq!(t["auto"], "Flag1", "{t}");
		assert_eq!(t["dup"], -3, "ERR_NAME_EXISTS: {t}");
		assert_eq!(t["badColor"], -10, "{t}");
		assert_eq!(t["longName"], -10, "名字 > 100: {t}");
		assert_eq!(t["badPos"], -10, "{t}");
		assert_eq!(t["posForm"], "F5", "{t}");
		let t = &infos[1];
		assert_eq!(t["names"], serde_json::json!(["F1", "F2", "F5", "Flag1"]), "旗经引擎持久化: {t}");
		assert_eq!(t["f2"], serde_json::json!([1, 3]), "{t}");
		assert_eq!(t["f5"], serde_json::json!([5, 5]), "secondaryColor 缺省 = color: {t}");
		assert_eq!(t["look"], serde_json::json!(["F2"]), "{t}");
		assert_eq!(t["find"], 4, "{t}");
		assert_eq!(t["byId"], true, "{t}");
		assert_eq!(t["setColor"], 0, "{t}");
		assert_eq!(t["setColorBad"], -10, "{t}");
		assert_eq!(t["move"], 0, "{t}");
		assert_eq!(t["remove"], 0, "{t}");
		let t = &infos[2];
		assert_eq!(t["names"], serde_json::json!(["F1", "F2", "F5"]), "{t}");
		assert_eq!(t["f1"], serde_json::json!([5, 5]), "{t}");
		assert_eq!(t["f2Room"], "W5N7", "旗可移到无视野房间: {t}");
		assert_eq!(t["f2RoomUndefined"], true, "{t}");
		assert_eq!(t["find"], 2, "{t}");
		assert_eq!(world.flags.len(), 3);
	});
}

/// 审计 #10/#15: look / lookFor / lookForAt 全类型、未知类型 ERR_INVALID_ARGS、lookAt 顺序、
/// RoomObject 参数、越界抛错、LOOK_TERRAIN 无需视野
#[test]
fn look_apis_match_official() {
	in_tokio(|| {
		let mut world = fidelity_world();
		{
			let room = world.rooms.get_mut("W5N8").unwrap();
			room.terrain.set(0, 0, 1);
			room.refresh_terrain_cache();
		}
		screeps_sim::engine::intents::create_dropped(&mut world, "W5N8", 20, 20, "energy", 40);
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8;
				const a = Game.creeps.A;
				const out = {};
				out.lookAtTypes = r.lookAt(a).map((i) => i.type);
				out.posLookTypes = a.pos.look().map((i) => i.type);
				out.creepByObj = r.lookForAt(LOOK_CREEPS, a).map((c) => c.name);
				out.energy = r.lookForAt(LOOK_ENERGY, 20, 20).map((e) => [e.resourceType, e.amount]);
				out.resources = a.pos.lookFor(LOOK_RESOURCES).length;
				out.unknown = r.lookForAt("bogus", 20, 20);
				out.empty = [LOOK_SOURCES, LOOK_MINERALS, LOOK_DEPOSITS, LOOK_STRUCTURES, LOOK_FLAGS, LOOK_CONSTRUCTION_SITES,
					LOOK_NUKES, LOOK_TOMBSTONES, LOOK_RUINS, LOOK_POWER_CREEPS].map((t) => r.lookForAt(t, 20, 20).length);
				out.terrain = r.lookForAt(LOOK_TERRAIN, 0, 0);
				out.mineral = r.lookForAt(LOOK_MINERALS, 40, 40).map((m) => m.mineralType);
				try { r.lookForAt(LOOK_CREEPS, 50, 10); out.oob = "no throw"; } catch (e) { out.oob = e.message; }
				out.farTerrain = new RoomPosition(3, 3, "W5N7").lookFor(LOOK_TERRAIN);
				try { new RoomPosition(3, 3, "W5N7").lookFor(LOOK_CREEPS); out.farLook = "no throw"; } catch (e) { out.farLook = e.message; }
				const area = r.lookForAtArea(LOOK_CREEPS, 19, 19, 21, 23, true).map((i) => i.creep.name).sort();
				out.area = area;
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(
			info["lookAtTypes"],
			serde_json::json!(["creep", "energy", "resource", "terrain"]),
			"官方 lookAt 顺序 (energy 与 resource 都列出掉落资源): {info}"
		);
		assert_eq!(info["posLookTypes"], info["lookAtTypes"], "{info}");
		assert_eq!(info["creepByObj"], serde_json::json!(["A"]), "RoomObject 参数: {info}");
		assert_eq!(info["energy"], serde_json::json!([["energy", 40]]), "{info}");
		assert_eq!(info["resources"], 1, "{info}");
		assert_eq!(info["unknown"], -10, "{info}");
		assert_eq!(info["empty"], serde_json::json!([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "{info}");
		assert_eq!(info["terrain"], serde_json::json!(["wall"]), "{info}");
		assert_eq!(info["mineral"], serde_json::json!(["H"]), "{info}");
		assert_eq!(info["oob"], "look coords are out of bounds", "{info}");
		assert_eq!(info["farTerrain"], serde_json::json!(["plain"]), "LOOK_TERRAIN 无需视野: {info}");
		assert_eq!(info["farLook"], "Could not access room W5N7", "{info}");
		assert_eq!(info["area"], serde_json::json!(["A", "B"]), "{info}");
	});
}

/// 审计 #12/#13: Room.Terrain / getRoomTerrain (未知房间抛错, 掩码, getRawBuffer(dest)) 与 Game.map
#[test]
fn terrain_and_map_match_official() {
	in_tokio(|| {
		let mut world = fidelity_world();
		{
			let room = world.rooms.get_mut("W5N7").unwrap();
			room.terrain.set(3, 4, 1);
			room.terrain.set(4, 4, 2);
			room.refresh_terrain_cache();
		}
		let bundle = r#"
			exports.loop = function() {
				const out = {};
				const t = Game.map.getRoomTerrain("W5N7");
				out.values = [t.get(3, 4), t.get(4, 4), t.get(5, 4)];
				out.viaRoom = new Room.Terrain("W5N7").get(3, 4);
				const dest = new Uint8Array(2500);
				out.sameDest = t.getRawBuffer(dest) === dest;
				out.destWall = dest[4 * 50 + 3];
				const copy = t.getRawBuffer();
				out.copyLen = copy.length;
				copy[0] = 1;
				out.copyIsolated = t.get(0, 0);
				try { new Room.Terrain("W40N40"); out.unknown = "no throw"; } catch (e) { out.unknown = e.message; }
				try { Game.map.getRoomTerrain("W40N40"); out.unknownMap = "no throw"; } catch (e) { out.unknownMap = e.message; }
				out.terrainAt = [Game.map.getTerrainAt(3, 4, "W5N7"), Game.map.getTerrainAt(new RoomPosition(4, 4, "W5N7")),
					Game.map.getTerrainAt(1, 1, "W40N40")];
				out.status = [Game.map.getRoomStatus("W5N8").status, Game.map.getRoomStatus("W40N40").status,
					Game.map.getRoomStatus("nope") === undefined];
				out.exits = Game.map.describeExits("W5N8");
				out.exitsUnknown = Game.map.describeExits("bad");
				out.dist = [Game.map.getRoomLinearDistance("W5N8", "W5N7"),
					Game.map.getRoomLinearDistance("E0N0", "W150N0"),
					Game.map.getRoomLinearDistance("E0N0", "W150N0", true)];
				console.log(JSON.stringify(out));
			};
		"#;
		let (info, _, _) = run_json(bundle, &world);
		assert_eq!(info["values"], serde_json::json!([1, 2, 0]), "{info}");
		assert_eq!(info["viaRoom"], 1, "{info}");
		assert_eq!(info["sameDest"], true, "{info}");
		assert_eq!(info["destWall"], 1, "{info}");
		assert_eq!(info["copyLen"], 2500, "{info}");
		assert_eq!(info["copyIsolated"], 0, "getRawBuffer() 返回副本: {info}");
		assert_eq!(info["unknown"], "Could not access room W40N40", "{info}");
		assert_eq!(info["unknownMap"], "Could not access room W40N40", "{info}");
		assert_eq!(info["terrainAt"], serde_json::json!(["wall", "swamp", null]), "{info}");
		assert_eq!(info["status"], serde_json::json!(["normal", "closed", true]), "{info}");
		assert_eq!(info["exits"], serde_json::json!({ "5": "W5N7", "7": "W6N8" }), "{info}");
		assert_eq!(info["exitsUnknown"], serde_json::Value::Null, "{info}");
		assert_eq!(info["dist"], serde_json::json!([1, 151, 49]), "continuous 按环绕世界取短边: {info}");
	});
}

/// 审计 #16/#17: rampart / 敌方结构归属来自引擎; Store toString 与 `in`; Game.notify 上限; getDirection(0,0); console 格式
#[test]
fn ownership_store_notify_console_match_official() {
	in_tokio(|| {
		let world = fidelity_world();
		let bundle = r#"
			exports.loop = function() {
				const r = Game.rooms.W5N8;
				const out = {};
				const ramps = r.find(FIND_STRUCTURES, { filter: { structureType: STRUCTURE_RAMPART } })
					.sort((a, b) => a.pos.x - b.pos.x);
				out.ramps = ramps.map((s) => [s.pos.x, s.my, s.owner && s.owner.username]);
				out.road = (() => { const s = r.lookForAt(LOOK_STRUCTURES, 10, 10)[0]; return [s.my === undefined, s.owner === undefined]; })();
				out.foeOwner = r.find(FIND_HOSTILE_CREEPS)[0].owner.username;
				out.neutralController = [Game.rooms.W6N8.controller.my === undefined, Game.rooms.W6N8.controller.owner === undefined];
				const st = Game.creeps.A.store;
				out.storeStr = String(st);
				out.inEmpty = "energy" in st;
				out.zero = st.energy;
				out.notify = [];
				for (let i = 0; i < 21; i++) out.notify.push(Game.notify("hi"));
				out.dir0 = Game.creeps.A.pos.getDirectionTo(Game.creeps.A.pos) === undefined;
				console.log(JSON.stringify(out));
				console.log(undefined, null, { a: 1 }, [1, 2], "s", 5, Game.creeps.A.pos);
			};
		"#;
		let (info, _, console) = run_json(bundle, &world);
		assert_eq!(
			info["ramps"],
			serde_json::json!([[12, true, "Nexus"], [14, false, "Invader"]]),
			"rampart 归属来自引擎: {info}"
		);
		assert_eq!(info["road"], serde_json::json!([true, true]), "{info}");
		assert_eq!(info["foeOwner"], "Invader", "{info}");
		assert_eq!(info["neutralController"], serde_json::json!([true, true]), "{info}");
		assert_eq!(info["storeStr"], "[store]", "{info}");
		assert_eq!(info["inEmpty"], false, "官方 Store 没有 has 陷阱: {info}");
		assert_eq!(info["zero"], 0, "{info}");
		let notify = info["notify"].as_array().unwrap();
		assert!(notify[..20].iter().all(|v| v == 0), "{info}");
		assert_eq!(notify[20], -8, "第 21 条 ERR_FULL: {info}");
		assert_eq!(info["dir0"], true, "{info}");
		assert_eq!(console[1], "undefined null [object Object] 1,2 s 5 [room W5N8 pos 20,20]");
	});
}
