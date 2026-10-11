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
	let intents = IntentList::parse(&result.intents_json);
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
			const source = creep.room.find(FIND_SOURCES)[0];
			const code = creep.moveTo(source);
			console.log(JSON.stringify({ code, x: creep.pos.x, y: creep.pos.y }));
		};
	"#;
	let mut pos = (25u8, 25u8);
	for _ in 0..12 {
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
		let info: serde_json::Value =
			serde_json::from_str(result.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info["code"], 0, "moveTo 应返回 OK: {info}");

		let intents = IntentList::parse(&result.intents_json);
		tick::run_tick(&mut world, movement, &intents);
		pos = (world.rooms["W5N8"].objects[&id].x, world.rooms["W5N8"].objects[&id].y);
	}
	// 12 tick × 3 MOVE 无负重 → 直线距离 6 格内必到达 source (18,22) 邻域
	let dist = (pos.0 as i32 - 18).abs().max((pos.1 as i32 - 22).abs());
	assert!(dist <= 1, "12 tick 后应到达 source 邻域, 实际 {pos:?} 距离 {dist}");
}

