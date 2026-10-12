//! 时间旅行回归: World 检查点的序列化往返, 以及
//! "检查点 + 意图重放" 与连续运行的世界状态逐位一致 (哈希对拍)。

use screeps_sim::engine::checkpoint::{self, Checkpoint};
use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;

const SCENARIO: &str = r#"{
	"name": "t",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "controller": { "level": 1, "pos": [28, 27] },
		  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }] }
	]
}"#;

fn setup() -> (World, Movement) {
	let scenario = Scenario::from_json(SCENARIO).unwrap();
	let world = build_world(&scenario).unwrap();
	(world, Movement::default())
}

/// 在 (18,22) 的 source 上放一个采集 creep, 返回 (creep id, source id)
fn add_harvester(world: &mut World) -> (u64, u64) {
	use screeps_sim::engine::objects::{creep_obj, BodyPart};
	use screeps_sim::engine::world::alloc_id;
	let room = world.rooms.get_mut("W5N8").unwrap();
	let source_id = *room
		.objects
		.iter()
		.find(|(_, o)| o.obj_type == "source")
		.map(|(id, _)| id)
		.expect("场景应有 source");
	let id = alloc_id(&mut world.next_id);
	let creep = creep_obj(
		id,
		"harvester",
		18,
		22,
		vec![BodyPart::new("work"), BodyPart::new("carry"), BodyPart::new("move")],
	);
	room.objects.insert(id, creep);
	world
		.rooms
		.get_mut("W5N8")
		.unwrap()
		.objects
		.get_mut(&id)
		.unwrap()
		.age_time = Some(world.tick + 1500);
	(id, source_id)
}

/// 30 tick 的合成意图流: 每 tick 采集 (偶数 tick 额外向右移动一格)
fn intent_stream(world: &mut World, ticks: usize) -> Vec<IntentList> {
	let (creep, source) = add_harvester(world);
	let mut stream = Vec::with_capacity(ticks);
	for t in 0..ticks {
		let json = if t % 2 == 0 {
			format!(
				r#"[{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}}]"#,
				creep, source
			)
		} else {
			format!(
				r#"[{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}},{{"o":"{:024x}","n":"move","d":{{"direction":3}}}}]"#,
				creep, source, creep
			)
		};
		stream.push(IntentList::parse(&json));
	}
	stream
}

/// 序列化往返后世界状态哈希不变 (terrain / 对象全量 / 缓存字段随同恢复)
#[test]
fn checkpoint_roundtrip_preserves_state_hash() {
	let (mut world, mut movement) = setup();
	let stream = intent_stream(&mut world, 5);
	for intents in &stream {
		tick::run_tick(&mut world, &mut movement, intents);
	}
	let hash_before = world.state_hash();
	// 世界应真有非平凡状态 (creep 背着能量), 否则测试没有区分度
	assert!(world.rooms["W5N8"]
		.objects
		.values()
		.any(|o| o.obj_type == "creep" && o.store.get("energy").copied().unwrap_or(0) > 0));

	let cp = Checkpoint::capture(&world, "{\"x\":1}", 1234.5, 100.0);
	let text = serde_json::to_string(&cp).unwrap();
	let cp2: Checkpoint = serde_json::from_str(&text).unwrap();
	assert_eq!(cp2.tick, world.tick);
	assert_eq!(cp2.memory, "{\"x\":1}");
	assert_eq!(cp2.cpu_bucket, 1234.5);
	let restored = cp2.restore_world();
	assert_eq!(restored.state_hash(), hash_before, "检查点往返必须逐位保持世界状态");
	for room in restored.rooms.values() {
		assert_eq!(room.terrain_json, room.terrain.to_string(), "terrain 缓存应随恢复刷新");
	}
}

/// save / latest_checkpoint / load / 文件名解析
#[test]
fn checkpoint_save_load_and_discovery() {
	let dir = std::env::temp_dir().join(format!("screeps-sim-cp-test-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	let (mut world, mut movement) = setup();
	let stream = intent_stream(&mut world, 12);
	for intents in &stream {
		tick::run_tick(&mut world, &mut movement, intents);
	}
	let hash = world.state_hash();
	let cp = Checkpoint::capture(&world, "{}", 9000.0, 100.0);
	let path = checkpoint::save(&dir, world.tick, &cp).unwrap();
	assert_eq!(checkpoint::parse_checkpoint_tick("cp-000012.json"), Some(12));
	assert_eq!(checkpoint::parse_checkpoint_tick("cp-12.json"), Some(12));
	assert_eq!(checkpoint::parse_checkpoint_tick("notes.md"), None);
	assert_eq!(checkpoint::parse_checkpoint_tick("cp-000012.json.tmp"), None);
	// ≤ at 语义: at 小于所有检查点 → None
	assert!(checkpoint::latest_checkpoint(&dir, 5).is_none());
	// at = world.tick → 正是刚落盘的那个
	let found = checkpoint::latest_checkpoint(&dir, world.tick).expect("应找到检查点");
	assert_eq!(found, path);
	let loaded = checkpoint::load(&found).unwrap();
	assert_eq!(loaded.tick, 12);
	assert_eq!(loaded.restore_world().state_hash(), hash, "落盘读回后状态不变");
	let _ = std::fs::remove_dir_all(&dir);
}

/// 核心对拍: 检查点 + 重放剩余意图 == 连续运行 (每 tick 状态哈希逐位一致)
#[test]
fn checkpoint_plus_replay_matches_continuous_run() {
	// A: 连续运行 30 tick, 记录每 tick 结束的状态哈希
	let (mut world, mut movement) = setup();
	let stream = intent_stream(&mut world, 30);
	let mut hashes = Vec::with_capacity(30);
	for intents in &stream {
		tick::run_tick(&mut world, &mut movement, intents);
		hashes.push(world.state_hash());
	}

	// B: 独立世界跑到 tick 10, 存检查点 (走完整 JSON 往返), 恢复后重放 10..30
	let (mut world2, mut movement2) = setup();
	let _ = intent_stream(&mut world2, 10); // 同样的初始对象
	for t in 0..10 {
		tick::run_tick(&mut world2, &mut movement2, &stream[t]);
	}
	assert_eq!(world2.tick, 10);
	let cp = Checkpoint::capture(&world2, "{}", 10000.0, 100.0);
	let text = serde_json::to_string(&cp).unwrap();
	let mut resumed: World = serde_json::from_str::<Checkpoint>(&text)
		.unwrap()
		.restore_world();
	assert_eq!(resumed.state_hash(), hashes[9], "检查点本身应与连续运行 t10 一致");

	for t in 10..30 {
		tick::run_tick(&mut resumed, &mut movement2, &stream[t]);
		assert_eq!(
			resumed.state_hash(),
			hashes[t],
			"tick {t}: 检查点+重放必须与连续运行逐位一致"
		);
	}
}
