//! CPU 看门狗: tickLimit 到期强制中断 V8 执行 (官方 "Script execution timed out ungracefully")。
//! 对齐官方语义: 中断的 tick 整 tick 丢弃 (意图不结算、Memory 改动不落盘), cpuUsed = tickLimit,
//! 且 isolate 必须被清理干净以便继续跑后续 tick。

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

/// deno_core 的 V8 isolate 需要 tokio 上下文
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

/// 首 tick 死循环 → 被看门狗中断; 次 tick 正常 → isolate 已恢复
#[test]
fn cpu_timeout_aborts_tick_and_recovers() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"
			globalThis.__n = 0;
			exports.loop = function() {
				globalThis.__n++;
				console.log("tick " + globalThis.__n);
				Game.rooms["W5N8"].createConstructionSite(10, 10, "road");
				Memory.touched = Game.time;
				if (globalThis.__n === 1) { for (;;) {} }
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let snap = snapshot(&world);

		let r1 = bot.run_tick_limited(snap.clone(), "{}".to_string(), Some(150.0)).unwrap();
		assert!(r1.timed_out, "首 tick 应被 CPU 看门狗中断");
		assert_eq!(r1.cpu_used, 150.0, "超时 tick 的 cpuUsed 应等于 tickLimit");
		assert!(r1.intents.entries.is_empty(), "官方在 isolate 被终止时丢失整个 tick 的意图");
		assert_eq!(r1.memory, "{}", "超时 tick 的 Memory 改动不落盘");
		assert!(
			r1.errors.iter().any(|e| e.contains("timed out ungracefully")),
			"应报官方同款超时错误: {:?}",
			r1.errors
		);
		assert!(
			r1.console.iter().any(|l| l.contains("tick 1")),
			"中断前已刷出的控制台输出应保留: {:?}",
			r1.console
		);

		// 看门狗必须清除 v8 终止标志, 否则后续 tick 会立刻被中断
		let r2 = bot.run_tick_limited(snap.clone(), "{}".to_string(), Some(5_000.0)).unwrap();
		assert!(!r2.timed_out, "第二 tick 应正常完成 (isolate 已恢复)");
		assert!(r2.memory.contains("touched"), "正常 tick 的 Memory 应落盘: {}", r2.memory);
		assert!(
			!r2.intents.entries.is_empty(),
			"正常 tick 应提交意图: {:?}",
			r2.intents.entries
		);
	});
}

/// 未超时不得误报
#[test]
fn cpu_no_timeout_when_under_limit() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"
			exports.loop = function() {
				Game.rooms["W5N8"].createConstructionSite(11, 11, "road");
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let r = bot.run_tick_limited(snapshot(&world), "{}".to_string(), Some(5_000.0)).unwrap();
		assert!(!r.timed_out, "轻量 tick 不应超时");
		assert!(r.errors.is_empty(), "不应产生错误: {:?}", r.errors);
		assert!(
			r.intents.entries.iter().any(|(_, n, _)| n == "createConstructionSite"),
			"意图应正常提交"
		);
	});
}

/// 不传 timeout 时保持原行为 (纯计量, 不中断)
#[test]
fn cpu_measure_only_without_timeout() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"exports.loop = function() { Memory.a = 1; };"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let r = bot.run_tick(snapshot(&world), "{}".to_string()).unwrap();
		assert!(!r.timed_out);
		assert!(r.memory.contains("\"a\":1"), "Memory 应落盘: {}", r.memory);
	});
}

/// 官方 intentCpu = 0.2: 每个 (对象, 意图) 组合首次提交计 0.2 CPU, 重复提交不重复计价,
/// 并计入当 tick 的 usedTime。40 只 creep 各提交 2 次 move → 只有 40 个唯一组合 → 8 CPU。
#[test]
fn intent_cpu_is_charged() {
	in_tokio(|| {
		let scenario = Scenario::from_json(&many_creeps_scenario(40)).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"
			exports.loop = function() {
				for (const c of Object.values(Game.creeps)) { c.move(1); c.move(3); }
			};
		"#;
		let r = BotRuntime::new(bundle, 1).unwrap().run_tick(snapshot(&world), "{}".to_string()).unwrap();
		assert_eq!(r.intents.entries.len(), 80, "应提交 80 条 move 意图");
		assert!(
			r.cpu_used >= 8.0,
			"40 个唯一 (对象, 意图) 组合应至少计 8 CPU, 实际 {}",
			r.cpu_used
		);
		assert!(
			r.cpu_used < 20.0,
			"重复提交同一组合不得重复计价 (否则会 ≥16 CPU), 实际 {}",
			r.cpu_used
		);
	});
}

/// 40 只 creep 的场景 (每只 1 条意图 → 便于按 0.2 CPU/条做量化断言)
fn many_creeps_scenario(n: usize) -> String {
	let creeps: Vec<String> = (0..n)
		.map(|i| {
			format!(
				r#"{{"name":"C{i}","body":["move","carry"],"pos":[{},{}]}}"#,
				5 + i % 40,
				20 + i / 40
			)
		})
		.collect();
	format!(
		r#"{{
			"name": "cpu",
			"seed": 1,
			"rooms": [
				{{ "name": "W5N8", "terrain": {{ "kind": "flat" }},
				   "sources": {{ "positions": [[18, 22]] }},
				   "controller": {{ "level": 1, "pos": [28, 27] }},
				   "structures": [{{ "type": "spawn", "pos": [25, 25], "energy": 300 }}],
				   "creeps": [{}] }}
			]
		}}"#,
		creeps.join(",")
	)
}
