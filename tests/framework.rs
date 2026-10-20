//! 运行框架语义: 全局重置 (isolate 重建、全局变量丢失、Memory 保留) 与场景加载校验。

use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::snapshot::{build_snapshot, CpuView, SnapshotOptions};
use screeps_sim::engine::world::World;
use screeps_sim::runtime::BotRuntime;

const ROOM: &str = r#"{
	"name": "t",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "controller": { "level": 1, "pos": [28, 27] },
		  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }] }
	]
}"#;

fn in_tokio<F: FnOnce()>(f: F) {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("tokio runtime");
	rt.block_on(async { f() });
}

fn snapshot(world: &World) -> String {
	let snap = build_snapshot(
		world,
		&SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: false,
			include_terrain: true,
		},
	);
	snap
}

fn console_json(r: &screeps_sim::runtime::TickResult) -> serde_json::Value {
	serde_json::from_str(r.console.first().map(String::as_str).unwrap_or("{}")).unwrap()
}

/// 全局重置: 全局变量清零, Memory 与内存段保留 (官方 global reset 语义)
#[test]
fn global_reset_clears_globals_but_keeps_memory_and_segments() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"
			exports.loop = function() {
				globalThis.__g = (globalThis.__g || 0) + 1;
				Memory.runs = (Memory.runs || 0) + 1;
				RawMemory.segments[7] = "seg" + Memory.runs;
				RawMemory.setActiveSegments([7]);
				console.log(JSON.stringify({ g: globalThis.__g, mem: Memory.runs }));
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1, "{}", "{}").unwrap();
		let snap = snapshot(&world);
		let mut memory = String::from("{}");

		let r1 = bot.run_tick(snap.clone(), memory.clone()).unwrap();
		assert_eq!(console_json(&r1)["g"], 1);
		assert_eq!(console_json(&r1)["mem"], 1);
		memory = r1.memory.clone();

		let r2 = bot.run_tick(snap.clone(), memory.clone()).unwrap();
		assert_eq!(console_json(&r2)["g"], 2, "未重置时全局变量应跨 tick 累加");
		assert_eq!(console_json(&r2)["mem"], 2, "Memory 应跨 tick 往返");
		memory = r2.memory.clone();

		bot.reset_global().unwrap();

		let r3 = bot.run_tick(snap.clone(), memory.clone()).unwrap();
		assert_eq!(console_json(&r3)["g"], 1, "重置后全局变量应从零开始");
		assert_eq!(console_json(&r3)["mem"], 3, "Memory 必须跨全局重置保留");
		assert_eq!(bot.segments().get(&7).map(String::as_str), Some("seg3"), "内存段也应保留");
	});
}

/// 场景校验: 各类非法输入都应在加载阶段报错 (带定位信息)
#[test]
fn scenario_validation_rejects_bad_input() {
	let cases: Vec<(&str, String, &str)> = vec![
		("空 rooms", r#"{"name":"x","rooms":[]}"#.to_string(), "rooms 不能为空"),
		("空 name", r#"{"name":"  ","rooms":[{"name":"W5N8"}]}"#.to_string(), "name 不能为空"),
		(
			"非法房名",
			r#"{"name":"x","rooms":[{"name":"W5N8x"}]}"#.to_string(),
			"非法房名",
		),
		(
			"重复房间",
			r#"{"name":"x","rooms":[{"name":"W5N8"},{"name":"W5N8"}]}"#.to_string(),
			"重复定义",
		),
		(
			"坐标越界",
			r#"{"name":"x","rooms":[{"name":"W5N8","controller":{"level":1,"pos":[60,10]}}]}"#
				.to_string(),
			"超出 0..49",
		),
		(
			"等级越界",
			r#"{"name":"x","rooms":[{"name":"W5N8","controller":{"level":9}}]}"#.to_string(),
			"超出 0..8",
		),
		(
			"未知部件",
			r#"{"name":"x","rooms":[{"name":"W5N8","creeps":[{"name":"a","body":["fly"],"pos":[1,1]}]}]}"#
				.to_string(),
			"未知部件",
		),
		(
			"单位重名",
			r#"{"name":"x","rooms":[{"name":"W5N8","creeps":[{"name":"a","body":["move"],"pos":[1,1]},{"name":"a","body":["move"],"pos":[2,2]}]}]}"#
				.to_string(),
			"重复",
		),
		(
			"未知结构",
			r#"{"name":"x","rooms":[{"name":"W5N8","structures":[{"type":"ufo","pos":[1,1]}]}]}"#
				.to_string(),
			"未知结构类型",
		),
		(
			"ticks=0",
			r#"{"name":"x","ticks":0,"rooms":[{"name":"W5N8"}]}"#.to_string(),
			"ticks 必须 > 0",
		),
		(
			"cpu.limit=0",
			r#"{"name":"x","cpu":{"limit":0},"rooms":[{"name":"W5N8"}]}"#.to_string(),
			"cpu.limit 必须 > 0",
		),
		(
			"globalResetEvery=0",
			r#"{"name":"x","globalResetEvery":0,"rooms":[{"name":"W5N8"}]}"#.to_string(),
			"globalResetEvery 必须 > 0",
		),
		(
			"terrain.raw 长度",
			r#"{"name":"x","rooms":[{"name":"W5N8","terrain":{"kind":"raw","data":"123"}}]}"#
				.to_string(),
			"terrain.raw 长度应为 2500",
		),
	];
	for (label, json, expect) in cases {
		let err = Scenario::from_json(&json).expect_err(&format!("{label} 应被拒绝"));
		assert!(err.contains(expect), "{label}: 期望含 {expect:?}, 实际 {err:?}");
	}
}

/// 同一场景 + 同一 seed 跑两遍: 最终快照 / Memory / 能量统计完全一致 (确定性);
/// 不同 seed 的随机轨迹应不同 (证明对比有效)。
#[test]
fn same_scenario_rerun_is_deterministic() {
	in_tokio(|| {
		let bundle = r#"
			exports.loop = function() {
				if (!Memory.r) Memory.r = [];
				Memory.r.push(Math.random().toFixed(8));
				const creep = Object.values(Game.creeps)[0];
				if (creep) {
					const src = creep.room.find(FIND_SOURCES)[0];
					if (creep.harvest(src) === ERR_NOT_IN_RANGE) creep.moveTo(src);
				}
			};
		"#;
		let run_once = |seed: u64| -> (String, String, u64, u64) {
			let scenario = Scenario::from_json(ROOM).unwrap();
			let mut world = build_world(&scenario).unwrap();
			let mut bot = BotRuntime::new(bundle, seed, "{}", "{}").unwrap();
			let mut movement = screeps_sim::engine::movement::Movement::default();
			let mut memory = String::from("{}");
			for _ in 0..8 {
				let snap = snapshot(&world);
				let r = bot.run_tick(snap.clone(), memory.clone()).unwrap();
				let intents = r.intents;
				screeps_sim::engine::tick::run_tick(&mut world, &mut movement, &intents);
				memory = r.memory;
			}
			(
				snapshot(&world),
				memory,
				world.energy_harvested,
				world.creeps_produced,
			)
		};
		let (snap_a, mem_a, harvested_a, _) = run_once(1);
		let (snap_b, mem_b, harvested_b, _) = run_once(1);
		assert_eq!(snap_a, snap_b, "同 seed 两次运行的最终快照应完全一致");
		assert_eq!(mem_a, mem_b, "同 seed 两次运行的 Memory 应完全一致");
		assert_eq!(harvested_a, harvested_b);

		let (_, mem_c, _, _) = run_once(99);
		assert_ne!(mem_a, mem_c, "不同 seed 的随机轨迹应不同 (否则对比无意义)");
	});
}

/// 合法场景 (含全局重置与 CPU 强制) 应能正常加载
#[test]
fn scenario_validation_accepts_valid_input() {
	let json = r#"{
		"name": "ok",
		"ticks": 100,
		"cpu": { "limit": 20, "bucket": 500, "enforce": true },
		"globalResetEvery": 500,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "controller": { "level": 8, "pos": [0, 49] },
			  "creeps": [{ "name": "a", "body": ["move", "carry", "work"], "pos": [49, 0] }],
			  "structures": [{ "type": "constructedWall", "pos": [1, 1] }] }
		]
	}"#;
	let s = Scenario::from_json(json).unwrap();
	assert!(s.cpu_enforce());
	assert_eq!(s.global_reset_every(), Some(500));
	assert_eq!(s.cpu_limit(), 20.0);
}

/// 世界状态哈希黄金回归: 受控意图流跑 12 tick 后的 state_hash 必须恒定。
/// 引擎任何结算顺序/数值变化都会改变哈希 —— 变化需有明确理由并更新此基线。
/// (FNV-1a 手工实现, 跨 Rust 版本/平台稳定; 见 World::state_hash)
#[test]
fn world_state_hash_golden() {
	const SCENE: &str = r#"{
		"name": "hash-golden",
		"seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "sources": { "positions": [[18, 22]] },
			  "controller": { "level": 1, "pos": [28, 27] },
			  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }],
			  "creeps": [{ "name": "w", "body": ["work", "move", "carry"], "pos": [24, 25] }] }
		]
	}"#;
	let scenario = Scenario::from_json(SCENE).unwrap();
	let run = || -> (u64, u64) {
		let mut world = build_world(&scenario).unwrap();
		let mut movement = screeps_sim::engine::movement::Movement::default();
		// 受控意图流: 单位交替左移/采集 (不依赖 JS, 纯引擎确定性)
		let creep_id = world
			.rooms
			.values()
			.next()
			.unwrap()
			.objects
			.values()
			.find(|o| o.obj_type == "creep")
			.map(|o| format!("{:024x}", o.id))
			.unwrap();
		for i in 0..12 {
			let intents = if i % 3 == 2 {
				screeps_sim::engine::intents::IntentList::parse(&format!(r#"[{{"o":"{creep_id}","n":"harvest"}}]"#))
			} else {
				screeps_sim::engine::intents::IntentList::parse(&format!(
					r#"[{{"o":"{creep_id}","n":"move","d":{{"direction":7}}}}]"#
				))
			};
			screeps_sim::engine::tick::run_tick(&mut world, &mut movement, &intents);
		}
		(world.tick, world.state_hash())
	};
	let (tick_a, hash_a) = run();
	let (tick_b, hash_b) = run();
	assert_eq!((tick_a, hash_a), (tick_b, hash_b), "同意图流两次运行哈希必须一致 (确定性)");
	assert_eq!(tick_a, 12);
	let golden = format!("0x{hash_a:016x}");
	println!("state_hash = {golden}");
	assert_eq!(
		golden,
		"0x886da1f4087983e9", // 黄金基线: 变更结算逻辑时有意更新
		"世界状态哈希发生变化 —— 引擎数值/顺序回归或基线需要有意更新"
	);
}
