//! RawMemory 语义: get/set 校验、内存段跨 tick 持久、激活列表按官方规则（不调用
//! setActiveSegments 即下一 tick 全部停用）以及段写入校验。

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
		},
	);
	serde_json::to_string(&snap).unwrap()
}

/// 段内容跨 tick 持久: tick1 写入 + 激活 → tick2 可读并续写
#[test]
fn memory_segments_persist_and_deactivate() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let snap = snapshot(&world);
		let bundle = r#"
			exports.loop = function() {
				globalThis.__n = (globalThis.__n || 0) + 1;
				const seg = RawMemory.segments[0];
				console.log(JSON.stringify({ n: globalThis.__n, seg0: seg === undefined ? null : seg, all: Object.keys(RawMemory.segments) }));
				RawMemory.segments[0] = (seg || "") + "t" + globalThis.__n;
				RawMemory.setActiveSegments([0, 1]);
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();

		// tick1: 段未激活 → 读不到, 写入 "t1", 激活 [0,1]
		let r1 = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		assert!(r1.console[0].contains("\"seg0\":null"), "首 tick 段未激活应读不到: {:?}", r1.console);
		assert_eq!(bot.segments().get(&0).map(String::as_str), Some("t1"), "段 0 应写入宿主");

		// tick2: 段已激活 → 读到 "t1", 追加为 "t1t2"
		let r2 = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		assert!(r2.console[0].contains("\"seg0\":\"t1\""), "次 tick 应读到上一 tick 写入: {:?}", r2.console);
		assert_eq!(bot.segments().get(&0).map(String::as_str), Some("t1t2"));
		assert_eq!(
			bot.segments().get(&1).map(String::as_str),
			Some(""),
			"激活但未写入的段按空串回存 (官方把激活段整段回传)"
		);
	});
}

/// setActiveSegments 未调用的下一 tick 段不可读 (官方 gotcha)
#[test]
fn segments_require_reactivation_each_tick() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let snap = snapshot(&world);
		let bundle = r#"
			globalThis.__n = 0;
			exports.loop = function() {
				globalThis.__n++;
				const before = RawMemory.segments[3];
				if (globalThis.__n === 1) {
					RawMemory.segments[3] = "hello";
					RawMemory.setActiveSegments([3]);
				}
				console.log(JSON.stringify({ n: globalThis.__n, seg3: before === undefined ? null : before }));
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let r1 = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		assert!(r1.console[0].contains("\"seg3\":null"), "tick1 未激活: {:?}", r1.console);
		let r2 = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		assert!(r2.console[0].contains("\"seg3\":\"hello\""), "tick2 已激活: {:?}", r2.console);
		let r3 = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		assert!(
			r3.console[0].contains("\"seg3\":null"),
			"tick3 未重新激活应读不到 (段内容仍保留在宿主): {:?}",
			r3.console
		);
		assert_eq!(bot.segments().get(&3).map(String::as_str), Some("hello"), "停用不删除段内容");
	});
}

/// 官方校验: 段 ID / 数量 / 类型 / 长度, 以及 RawMemory.set 的类型与 2MB 限制
#[test]
fn raw_memory_validation_matches_official() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let snap = snapshot(&world);
		let bundle = r#"
			function tryCatch(fn) {
				try { fn(); return "ok"; } catch (e) { return e.message; }
			}
			exports.loop = function() {
				const out = {
					notString: tryCatch(() => RawMemory.set(123)),
					badId: tryCatch(() => RawMemory.setActiveSegments([100])),
					notArray: tryCatch(() => RawMemory.setActiveSegments("x")),
					tooMany: tryCatch(() => RawMemory.setActiveSegments([0,1,2,3,4,5,6,7,8,9,10])),
					nonStringSeg: tryCatch(() => { RawMemory.segments[5] = 42; }),
					setOk: tryCatch(() => RawMemory.set('{"a":1}')),
					memAfterSet: Memory.a,
				};
				// 触发段写回校验 (非字符串段)
				RawMemory.setActiveSegments([]);
				console.log(JSON.stringify(out));
			};
		"#;
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let r = bot.run_tick(snap.clone(), "{}".to_string()).unwrap();
		let info: serde_json::Value =
			serde_json::from_str(r.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info["notString"], "Raw memory value is not a string");
		assert_eq!(info["badId"], "\"100\" is not a valid segment ID");
		assert_eq!(info["notArray"], "\"x\" is not an array");
		assert_eq!(info["tooMany"], "Only 10 memory segments can be active at the same time");
		assert_eq!(info["setOk"], "ok");
		assert_eq!(info["memAfterSet"], 1, "RawMemory.set 后 Memory 应反映新内容");
		assert!(
			r.errors.iter().any(|e| e.contains("is not a string")),
			"非字符串段应在 tick 末尾被拒绝: {:?}",
			r.errors
		);
		assert!(bot.segments().get(&5).is_none(), "非法段不应写入宿主");
	});
}

/// issue #1 §3.4: Memory 是 globalThis 上的可配置属性 — strict 模式下
/// `delete globalThis.Memory` 不抛错; 底层数据保留并照常保存; 下一 tick 访问器自动重建。
#[test]
fn memory_is_configurable_and_survives_delete() {
	in_tokio(|| {
		let scenario = Scenario::from_json(ROOM).unwrap();
		let world = build_world(&scenario).unwrap();
		let bundle = r#"
			exports.loop = function() {
				const desc = Object.getOwnPropertyDescriptor(globalThis, "Memory");
				if (!Memory.ran) {
					Memory.ran = 1;
					const del = delete globalThis.Memory;
					console.log(JSON.stringify({ configurable: desc && desc.configurable, del, gone: globalThis.Memory === undefined }));
				} else {
					Memory.ran += 1;
					console.log(JSON.stringify({ configurable: desc && desc.configurable, ran: Memory.ran }));
				}
			};
		"#;
		let snap = snapshot(&world);
		let mut bot = BotRuntime::new(bundle, 1).unwrap();
		let memory = String::from("{}");

		let r1 = bot.run_tick(snap.clone(), memory.clone()).unwrap();
		let info: serde_json::Value =
			serde_json::from_str(r1.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info["configurable"], true, "Memory 必须是可配置属性: {info}");
		assert_eq!(info["del"], true, "delete 不应抛错且返回 true: {info}");
		assert_eq!(info["gone"], true, "删除后属性应消失: {info}");
		assert!(
			r1.memory.contains("\"ran\":1"),
			"删除属性不应丢弃底层数据 (官方 rawMemory._parsed 语义): {}",
			r1.memory
		);

		let r2 = bot.run_tick(snap.clone(), r1.memory.clone()).unwrap();
		let info2: serde_json::Value =
			serde_json::from_str(r2.console.first().map(String::as_str).unwrap_or("{}")).unwrap();
		assert_eq!(info2["configurable"], true, "下一 tick 访问器应自动重建: {info2}");
		assert_eq!(info2["ran"], 2, "Memory 应跨 tick 往返: {info2}");
		assert!(r2.errors.is_empty(), "不应有任何 bot 异常: {:?}", r2.errors);
	});
}
