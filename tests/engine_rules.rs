//! 机制回归测试: 用受控意图直接驱动结算引擎, 锁定与官方规则对齐的关键数值。
//! 不经过 V8 —— 快且完全确定性; JS 侧 (PathFinder 跨房) 另见 multiroom_js.rs。

use nexus_sim::engine::intents::IntentList;
use nexus_sim::engine::movement::Movement;
use nexus_sim::engine::scenario::{build_world, Scenario};
use nexus_sim::engine::tick;
use nexus_sim::engine::world::alloc_id;
use nexus_sim::engine::world::World;

const TWO_ROOMS: &str = r#"{
	"name": "t",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "sources": { "positions": [[18, 22]] },
		  "controller": { "level": 1, "pos": [28, 27] },
		  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }] },
		{ "name": "W4N8", "terrain": { "kind": "flat" } }
	]
}"#;

fn setup(json: &str) -> (World, Movement) {
	let scenario = Scenario::from_json(json).unwrap();
	let world = build_world(&scenario).unwrap();
	(world, Movement::default())
}

fn find_creep(world: &World, name: &str) -> (u64, u8, u8) {
	for room in world.rooms.values() {
		for obj in room.objects.values() {
			if obj.obj_type == "creep" && obj.name.as_deref() == Some(name) {
				return (obj.id, obj.x, obj.y);
			}
		}
	}
	panic!("creep {name} 不存在");
}

fn add_creep(world: &mut World, name: &str, x: u8, y: u8, body: &[&str]) -> u64 {
	use nexus_sim::engine::objects::{creep_obj, BodyPart};
	let id = alloc_id(&mut world.next_id);
	let body: Vec<BodyPart> = body
		.iter()
		.map(|p| BodyPart { part_type: p.to_string(), hits: 100 })
		.collect();
	let creep = creep_obj(id, name, x, y, body);
	let room = world.rooms.get_mut("W5N8").unwrap();
	room.objects.insert(id, creep);
	// 补 ageTime (prime 只在场景构建时跑)
	world
		.rooms
		.get_mut("W5N8")
		.unwrap()
		.objects
		.get_mut(&id)
		.unwrap()
		.age_time = Some(world.tick + 1500);
	id
}

/// 掉落能量衰减: 每 tick ⌈n/1000⌉, 1000 能量恰好 ~500+ tick 归零
#[test]
fn dropped_energy_decay() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let id = alloc_id(&mut world.next_id);
	let pile = nexus_sim::engine::objects::dropped_energy(id, 20, 20, 1000);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(id, pile);
	let empty = IntentList { entries: vec![] };
	for _ in 0..1100 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let room = &world.rooms["W5N8"];
	assert!(
		!room.objects.contains_key(&id),
		"1100 tick 后 1000 能量应已衰减完 (每 tick ⌈n/1000⌉)"
	);
}

/// Source 再生: 低于容量起 300t 计时, 到点整段回满 (不是清空后计时)
#[test]
fn source_regen_from_depletion_start() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let empty = IntentList { entries: vec![] };
	// 只采 1 tick (从 3000 降到 2998): 计时起点应是这一 tick
	for _ in 0..5 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let source_id = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "source")
		.unwrap()
		.id;
	let room = world.rooms.get_mut("W5N8").unwrap();
	let source = room.objects.get_mut(&source_id).unwrap();
	source.store.insert("energy".into(), 2998);
	source.next_regeneration_time = None; // 模拟"刚被采过一口"

	for _ in 0..298 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let energy = world.rooms["W5N8"].objects[&source_id]
		.store
		.get("energy")
		.copied()
		.unwrap();
	assert_eq!(energy, 2998, "未到 300t 不应回满");
	for _ in 0..3 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let energy = world.rooms["W5N8"].objects[&source_id]
		.store
		.get("energy")
		.copied()
		.unwrap();
	assert_eq!(energy, 3000, "300t 到点整段回满");
}

/// 疲劳: 1:1:1 身体平原重载移动 = 每 2 tick 一步 (负重 4, 恢复 2/t)
#[test]
fn fatigue_plain_movement() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let id = add_creep(&mut world, "H1", 25, 25, &["work", "carry", "move"]);
	// 装满货 → 负重 2 (work + 折算 1 carry)
	{
		let room = world.rooms.get_mut("W5N8").unwrap();
		room.objects.get_mut(&id).unwrap().store.insert("energy".into(), 50);
	}
	// 连续 4 tick 提交 move(RIGHT): 期望步进 2 格
	let move_json = format!(r#"[{{"o":"{:024x}","n":"move","d":{{"direction":3}}}}]"#, id);
	for _ in 0..4 {
		let intents = IntentList::parse(&move_json);
		tick::run_tick(&mut world, &mut movement, &intents);
	}
	let (_, x, _) = find_creep(&world, "H1");
	assert_eq!(x, 27, "重载 1:1:1 平原 4 tick 应走 2 格, 实际 x={x}");
}

/// 出口格即时越房: 走到 x=49 后同 tick 镜像进入东邻房 W4N8 x=0; 停留边缘会被弹回
#[test]
fn edge_crossing_transfers_same_tick() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let id = add_creep(&mut world, "S1", 47, 25, &["move", "move", "move"]);
	// 3 个 MOVE, 无负重: 每步 0 疲劳 (weight 0) → 可连续走
	let move_json = format!(r#"[{{"o":"{:024x}","n":"move","d":{{"direction":3}}}}]"#, id);
	for _ in 0..2 {
		let intents = IntentList::parse(&move_json);
		tick::run_tick(&mut world, &mut movement, &intents);
	}
	// t2: 到达 (49,25), tick 结束即越房到 W4N8 (0,25)
	assert!(
		!world.rooms["W5N8"].objects.contains_key(&id),
		"到达出口格的 tick 末即越房"
	);
	let obj = world.rooms["W4N8"].objects.get(&id).expect("应已进入 W4N8");
	assert_eq!((obj.x, obj.y), (0, 25), "镜像落位 (0,25), 实际 ({},{})", obj.x, obj.y);
	// t3: 无移动意图, 站在 (0,25) 边缘 → 官方规则: 出口格不能停留, 被弹回原房
	let empty = IntentList { entries: vec![] };
	tick::run_tick(&mut world, &mut movement, &empty);
	let obj = world.rooms["W5N8"].objects.get(&id).expect("出口格停留应被弹回 W5N8");
	assert_eq!((obj.x, obj.y), (49, 25), "弹回落位 (49,25)");
}

/// 控制器升级与降级: 200 进度到 RCL2; 长期不升级触发降级并保留 90% 进度
#[test]
fn controller_level_and_downgrade() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let id = add_creep(&mut world, "U1", 27, 26, &["work", "carry", "move", "work", "work"]);
	// 装满能量 (150), 3 WORK → 每 tick 注 3 进度
	{
		let room = world.rooms.get_mut("W5N8").unwrap();
		room.objects.get_mut(&id).unwrap().store.insert("energy".into(), 300);
	}
	let ctrl_id = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap()
		.id;
	let upgrade_json = format!(
		r#"[{{"o":"{:024x}","n":"upgradeController","d":{{"id":"{:024x}"}}}}]"#,
		id, ctrl_id
	);
	for _ in 0..68 {
		let intents = IntentList::parse(&upgrade_json);
		tick::run_tick(&mut world, &mut movement, &intents);
	}
	let level = world.rooms["W5N8"].objects[&ctrl_id].level;
	assert_eq!(level, 2, "68 tick × 3 = 204 ≥ 200 应升 RCL2");

	// 停止升级: RCL2 降级计时 10000t, 到点降回 RCL1 且进度保留 90%
	let empty = IntentList { entries: vec![] };
	for _ in 0..10_100 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let controller = &world.rooms["W5N8"].objects[&ctrl_id];
	assert_eq!(controller.level, 1, "10000t 不升级应降级");
	assert_eq!(controller.progress, 184, "降级后进度 += 90% × 200 = 180 (4+180)");
}
