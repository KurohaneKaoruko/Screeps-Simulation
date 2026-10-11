//! 机制回归测试: 用受控意图直接驱动结算引擎, 锁定与官方规则对齐的关键数值。
//! 不经过 V8 —— 快且完全确定性; JS 侧 (PathFinder 跨房 / moveTo) 另见 shim_apis.rs。

use nexus_sim::engine::constants;
use nexus_sim::engine::intents::IntentList;
use nexus_sim::engine::movement::Movement;
use nexus_sim::engine::objects;
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
/// 跑 6 tick: 每 2 tick 一步 → 3 格; 每 3 tick 一步 → 2 格, 可区分两种步频。
#[test]
fn fatigue_plain_movement() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let id = add_creep(&mut world, "H1", 25, 25, &["work", "carry", "move"]);
	// 装满货 → 负重 2 (work + 折算 1 carry)
	{
		let room = world.rooms.get_mut("W5N8").unwrap();
		room.objects.get_mut(&id).unwrap().store.insert("energy".into(), 50);
	}
	// 连续 6 tick 提交 move(RIGHT): 期望步进 3 格 (每 2 tick 一步)
	let move_json = format!(r#"[{{"o":"{:024x}","n":"move","d":{{"direction":3}}}}]"#, id);
	for _ in 0..6 {
		let intents = IntentList::parse(&move_json);
		tick::run_tick(&mut world, &mut movement, &intents);
	}
	let (_, x, _) = find_creep(&world, "H1");
	assert_eq!(x, 28, "重载 1:1:1 平原 6 tick 应走 3 格 (每 2 tick 一步), 实际 x={x}");
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
	// 3 WORK 负责升级; 6 CARRY (300 容量) 保证塞进去的 300 能量不超容量
	let id = add_creep(
		&mut world,
		"U1",
		27,
		26,
		&["work", "work", "work", "carry", "carry", "carry", "carry", "carry", "carry", "move"],
	);
	// 装满能量 (300 = 6 × 50), 3 WORK → 每 tick 注 3 进度
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

	// 停止升级: RCL2 的降级计时 = DOWNGRADE[2]/2 = 5000 tick (升级时刷新, 上限 +10001)
	let empty = IntentList { entries: vec![] };
	for _ in 0..5_000 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	assert_eq!(
		world.rooms["W5N8"].objects[&ctrl_id].level,
		2,
		"RCL2 降级计时约 5000t, 5000 tick 时不应降级"
	);
	for _ in 0..6_000 {
		tick::run_tick(&mut world, &mut movement, &empty);
	}
	let controller = &world.rooms["W5N8"].objects[&ctrl_id];
	assert_eq!(controller.level, 1, "长时间不升级应降回 RCL1");
	assert_eq!(controller.progress, 184, "降级后进度 += 90% × 200 = 180 (4+180)");
}

// ── 以下为 issue #1「经济主链路」回归用例 ──

/// 往房间塞一个结构 (测试辅助)
fn add_structure(world: &mut World, room: &str, st: &str, x: u8, y: u8) -> u64 {
	let id = alloc_id(&mut world.next_id);
	let mut obj = objects::structure(id, st, x, y, true);
	obj.energy_capacity = constants::structure_energy_capacity(st, 1);
	world.rooms.get_mut(room).unwrap().objects.insert(id, obj);
	id
}

fn first_source(world: &World) -> u64 {
	world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "source")
		.unwrap()
		.id
}

fn store_of(world: &World, id: u64) -> u32 {
	world.rooms["W5N8"].objects[&id].store.get("energy").copied().unwrap_or(0)
}

fn set_store(world: &mut World, id: u64, energy: u32) {
	world
		.rooms
		.get_mut("W5N8")
		.unwrap()
		.objects
		.get_mut(&id)
		.unwrap()
		.store
		.insert("energy".into(), energy);
}

/// issue#1 §1.1: creep→creep / creep→link 的能量转移必须按官方 capacityForResource 生效
#[test]
fn transfer_capacity_for_creep_and_link() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "TA", 20, 20, &["carry", "move"]);
	let b = add_creep(&mut world, "TB", 21, 20, &["carry", "move"]);
	let link = add_structure(&mut world, "W5N8", "link", 20, 21);
	set_store(&mut world, a, 50);

	let json = format!(
		r#"[{{"o":"{:024x}","n":"transfer","d":{{"id":"{:024x}","resourceType":"energy","amount":50}}}}]"#,
		a, b
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, b), 50, "creep→creep 转移容量应为 CARRY×50");
	assert_eq!(store_of(&world, a), 0);

	set_store(&mut world, a, 50);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"transfer","d":{{"id":"{:024x}","resourceType":"energy","amount":50}}}}]"#,
		a, link
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, link), 50, "creep→link 转移容量应为 LINK_CAPACITY (800)");
}

/// issue#1 §1.2: 掉落资源先装入同格 hits>0 的容器, 装不下的部分才落地
#[test]
fn dropped_resource_fills_container_first() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let container = add_structure(&mut world, "W5N8", "container", 20, 20);
	let c = add_creep(&mut world, "D1", 20, 20, &["carry", "move"]);
	set_store(&mut world, c, 50);

	let json = format!(r#"[{{"o":"{:024x}","n":"drop","d":{{"resourceType":"energy"}}}}]"#, c);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, container), 50, "drop 应先进同格容器");
	assert!(
		!world.rooms["W5N8"].objects.values().any(|o| o.obj_type == "energy"),
		"容器装得下时不应产生地上的堆"
	);
}

/// issue#1 §1.2: 容器装不下的部分仍然落地 (采集溢出路径)
#[test]
fn container_overflow_drops_remainder() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let container = add_structure(&mut world, "W5N8", "container", 20, 20);
	set_store(&mut world, container, constants::CONTAINER_CAPACITY - 10);
	let c = add_creep(&mut world, "D2", 20, 20, &["carry", "move"]);
	set_store(&mut world, c, 50);

	let json = format!(r#"[{{"o":"{:024x}","n":"drop","d":{{"resourceType":"energy"}}}}]"#, c);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, container), constants::CONTAINER_CAPACITY, "容器应先被填满");
	let pile = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "energy")
		.expect("装不下的 40 应落地");
	// 同一 tick 的掉落衰减也会作用到新建的堆: 40 - ⌈40/1000⌉ = 39
	assert_eq!(pile.store.get("energy").copied().unwrap_or(0), 39);
}

/// issue#1 §1.3: 部分拾取只扣减数量, 减到 0 才删除整堆
#[test]
fn pickup_partial_keeps_remainder() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let pile_id = alloc_id(&mut world.next_id);
	world
		.rooms
		.get_mut("W5N8")
		.unwrap()
		.objects
		.insert(pile_id, objects::dropped_energy(pile_id, 20, 20, 1000));
	let c = add_creep(&mut world, "P1", 20, 20, &["carry", "move"]);

	let json = format!(r#"[{{"o":"{:024x}","n":"pickup","d":{{"id":"{:024x}"}}}}]"#, c, pile_id);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert!(room.objects.contains_key(&pile_id), "50 容量拾 1000 的堆不应删掉整堆");
	let left = room.objects[&pile_id].store.get("energy").copied().unwrap_or(0);
	assert!((900..=950).contains(&left), "剩余应约 950 (扣 1 衰减), 实际 {left}");
	assert_eq!(store_of(&world, c), 50);
}

/// issue#1 §1.5: 同 tick 有 build/repair 时 harvest 不执行 (官方 checkPriorities)
#[test]
fn build_suppresses_harvest_same_tick() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let source_id = first_source(&world);
	// (19,22) 与 source (18,22) 相邻; 工地 (20,22) 在建造范围 3 内
	let c = add_creep(&mut world, "X1", 19, 22, &["work", "carry", "move"]);
	set_store(&mut world, c, 50);
	let site_id = alloc_id(&mut world.next_id);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(
		site_id,
		objects::construction_site(site_id, "extension", 20, 22, 3000),
	);

	let json = format!(
		r#"[{{"o":"{:024x}","n":"build","d":{{"id":"{:024x}"}}}},{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}}]"#,
		c, site_id, c, source_id
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert_eq!(
		room.objects[&source_id].store.get("energy").copied().unwrap_or(0),
		3000,
		"有 build 时 harvest 不应执行"
	);
	assert_eq!(room.objects[&site_id].progress, 5, "build 应正常执行 (1 WORK × 5)");
}

/// issue#1 §1.5: 意图顺序 transfer 先于 harvest — 满载矿工同 tick 交接不溢出到地上
#[test]
fn transfer_runs_before_harvest_same_tick() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let source_id = first_source(&world);
	let miner = add_creep(&mut world, "M1", 19, 22, &["work", "carry", "move"]);
	let hauler = add_creep(&mut world, "M2", 20, 22, &["carry", "move"]);
	set_store(&mut world, miner, 50);

	let json = format!(
		r#"[{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}},{{"o":"{:024x}","n":"transfer","d":{{"id":"{:024x}","resourceType":"energy","amount":50}}}}]"#,
		miner, source_id, miner, hauler
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, hauler), 50, "应先 transfer 把 50 交给搬运工");
	assert_eq!(store_of(&world, miner), 2, "腾出容量后 harvest 的 2 应留在身上");
	assert!(
		!world.rooms["W5N8"].objects.values().any(|o| o.obj_type == "energy"),
		"顺序正确时不应溢出到地上"
	);
}

/// issue#1 §1.17: 超出 RCL 上限的 extension 被标记 off, 不计入能量统计
#[test]
fn spawn_extension_off_beyond_rcl_limit() {
	let json = r#"{
		"name": "t", "seed": 1,
		"rooms": [{
			"name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 1, "pos": [28, 27] },
			"structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 300 },
				{ "type": "extension", "pos": [24, 25], "energy": 50 },
				{ "type": "extension", "pos": [26, 25], "energy": 50 }
			]
		}]
	}"#;
	let (world, _) = setup(json);
	let room = &world.rooms["W5N8"];
	assert_eq!(room.energy_capacity_available(), 300, "RCL1 不含 extension, 容量只算 spawn");
	assert_eq!(room.energy_available(), 300, "被停用的 extension 能量不计入");
	let off = room
		.objects
		.values()
		.filter(|o| o.obj_type == "extension" && o.off)
		.count();
	assert_eq!(off, 2, "RCL1 extension 上限 0, 两个都应为 off");
}

/// issue#1 §4: GCL 需求公式为累计 (L)^2.4 × 1e6
#[test]
fn gcl_progress_total_matches_official() {
	assert_eq!(constants::gcl_progress_total(1), 1_000_000, "GCL1→2 = 1e6");
	assert_eq!(constants::gcl_progress_total(2), 5_278_031, "GCL2→3 = 2^2.4 × 1e6");
}

/// issue#1 §1.14: 引擎记录官方结构的事件日志, 且每 tick 清空
#[test]
fn event_log_records_economy_events() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let source_id = first_source(&world);
	let c = add_creep(&mut world, "E1", 19, 22, &["work", "carry", "move"]);
	let json = format!(r#"[{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}}]"#, c, source_id);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let events = &world.rooms["W5N8"].events;
	let harvest: Vec<_> = events.iter().filter(|e| e.event == 5).collect();
	assert_eq!(harvest.len(), 1, "应记录 1 条 EVENT_HARVEST");
	assert_eq!(harvest[0].data["amount"], 2);

	let empty = IntentList { entries: vec![] };
	tick::run_tick(&mut world, &mut movement, &empty);
	assert!(world.rooms["W5N8"].events.is_empty(), "事件日志每 tick 清空");
}

/// issue#1 §1.2: 单位死亡时携带资源先入同格容器, 剩余才进墓碑
#[test]
fn death_resources_go_to_container() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let container = add_structure(&mut world, "W5N8", "container", 20, 20);
	let c = add_creep(&mut world, "S9", 20, 20, &["carry", "move"]);
	set_store(&mut world, c, 50);

	let json = format!(r#"[{{"o":"{:024x}","n":"suicide","d":{{}}}}]"#, c);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	// 携带 50 + 身体资源 2 部件 × min(125, 50×0.2)=10 → 70 (官方 _die.js: 自杀按 CREEP_CORPSE_RATE 回收)
	assert_eq!(store_of(&world, container), 70, "死亡资源 (携带+身体) 应先入同格容器");
	let tomb = room
		.objects
		.values()
		.find(|o| o.obj_type == "tombstone")
		.expect("应留下墓碑");
	assert_eq!(tomb.store.get("energy").copied().unwrap_or(0), 0, "容器装下的部分不再进墓碑");
}

// ── 以下为 issue #1「多房间 / 数值 / 次要规则 / API」回归用例 ──

const RCL3_ROOM: &str = r#"{
	"name": "t3",
	"seed": 1,
	"rooms": [
		{ "name": "W5N8", "terrain": { "kind": "flat" },
		  "controller": { "level": 3, "pos": [28, 27] },
		  "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }] }
	]
}"#;

fn add_creep_in(world: &mut World, room: &str, name: &str, x: u8, y: u8, body: &[&str]) -> u64 {
	use nexus_sim::engine::objects::{creep_obj, BodyPart};
	let id = alloc_id(&mut world.next_id);
	let body: Vec<BodyPart> = body
		.iter()
		.map(|p| BodyPart { part_type: p.to_string(), hits: 100 })
		.collect();
	let creep = creep_obj(id, name, x, y, body);
	let room = world.rooms.get_mut(room).unwrap();
	room.objects.insert(id, creep);
	room.objects.get_mut(&id).unwrap().age_time = Some(world.tick + 1500);
	id
}

fn spawn_of(world: &World, room: &str) -> u64 {
	world.rooms[room]
		.objects
		.values()
		.find(|o| o.obj_type == "spawn")
		.unwrap()
		.id
}

/// issue#1 §2.1: 每个房间各自仲裁, 后处理房间不得取消前面房间的移动
#[test]
fn multi_room_movement_isolated() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep_in(&mut world, "W5N8", "A", 20, 20, &["move"]);
	let b = add_creep_in(&mut world, "W4N8", "B", 20, 20, &["move"]);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"move","d":{{"direction":3}}}},{{"o":"{:024x}","n":"move","d":{{"direction":3}}}}]"#,
		a, b
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!((world.rooms["W5N8"].objects[&a].x, world.rooms["W5N8"].objects[&a].y), (21, 20));
	assert_eq!(
		(world.rooms["W4N8"].objects[&b].x, world.rooms["W4N8"].objects[&b].y),
		(21, 20),
		"两个房间的移动都应在同一 tick 生效"
	);
}

/// issue#1 §2.2: 交换对象 (rate1) 应优先于同格的第三方抢位
#[test]
fn swap_priority_resolves_head_on() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	// 避开 (25,25) 的 spawn, 否则目标格会被阻挡结构占住导致意图不登记
	let a = add_creep_in(&mut world, "W5N8", "A", 20, 20, &["move", "carry"]);
	let b = add_creep_in(&mut world, "W5N8", "B", 21, 20, &["move", "carry"]);
	// C 的 MOVE 部件更多 (rate4 更高), 若无 rate1 会抢走 (21,20)
	let c = add_creep_in(&mut world, "W5N8", "C", 22, 20, &["move", "move", "move", "carry"]);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"move","d":{{"direction":3}}}},{{"o":"{:024x}","n":"move","d":{{"direction":7}}}},{{"o":"{:024x}","n":"move","d":{{"direction":7}}}}]"#,
		a, b, c
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert_eq!((room.objects[&a].x, room.objects[&a].y), (21, 20), "A 与 B 交换应优先");
	assert_eq!((room.objects[&b].x, room.objects[&b].y), (20, 20));
	assert_eq!((room.objects[&c].x, room.objects[&c].y), (22, 20), "C 应被挡下");
}

/// issue#1 §3.9: 可见性来自带 user 的对象; 只有路/容器的房间不算可见
#[test]
fn visibility_requires_user_object() {
	let (mut world, _) = setup(TWO_ROOMS);
	assert!(world.rooms["W5N8"].visible(), "有 controller/spawn 应可见");
	assert!(!world.rooms["W4N8"].visible(), "空房不可见");
	add_structure(&mut world, "W4N8", "road", 10, 10);
	add_structure(&mut world, "W4N8", "container", 11, 10);
	assert!(!world.rooms["W4N8"].visible(), "只有路和容器仍不算可见");
}

/// issue#1 §4.2: 墙上的路造价 ×150
#[test]
fn road_on_wall_construction_cost_ratio() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	world.rooms.get_mut("W5N8").unwrap().terrain.set(5, 5, 1);
	let json = r#"[{"o":"room","n":"createConstructionSite","d":{"room":"W5N8","x":5,"y":5,"structureType":"road"}}]"#;
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(json));
	let site = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "constructionSite")
		.expect("墙上应能建路");
	assert_eq!(site.progress_total, 300 * constants::CONSTRUCTION_COST_ROAD_WALL_RATIO);
}

/// issue#1 §4.3: 新建容器首次衰减计时为 +100
#[test]
fn container_first_decay_is_100() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let site_id = alloc_id(&mut world.next_id);
	let mut site = objects::construction_site(site_id, "container", 10, 10, 5000);
	site.progress = 4995;
	world.rooms.get_mut("W5N8").unwrap().objects.insert(site_id, site);
	let c = add_creep_in(&mut world, "W5N8", "B1", 10, 11, &["work", "carry"]);
	set_store(&mut world, c, 100);
	let json = format!(r#"[{{"o":"{:024x}","n":"build","d":{{"id":"{:024x}"}}}}]"#, c, site_id);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let container = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "container")
		.expect("容器应建成");
	assert_eq!(container.next_decay_time, Some(100), "新建容器首次衰减在 +100");
}

/// issue#1 §4.4: rampart 每 100t 减 300, hitsMax 按 RCL
#[test]
fn rampart_decays_and_hits_max_by_rcl() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let id = add_structure(&mut world, "W5N8", "rampart", 10, 10);
	{
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&id).unwrap();
		o.hits = 1000;
		o.next_decay_time = Some(0);
	}
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	let o = &world.rooms["W5N8"].objects[&id];
	assert_eq!(o.hits, 700, "rampart 每 100t 减 300");
	assert_eq!(o.hits_max, constants::rampart_hits_max(3), "hitsMax = RAMPART_HITS_MAX[RCL3]");
}

/// issue#1 §4.6: 障碍类工地被同格 creep 挡住时不施工
#[test]
fn build_blocked_by_creep_on_obstacle_site() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let site_id = alloc_id(&mut world.next_id);
	let site = objects::construction_site(site_id, "spawn", 10, 10, 15000);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(site_id, site);
	add_creep_in(&mut world, "W5N8", "X1", 10, 10, &["work", "carry"]);
	let c = add_creep_in(&mut world, "W5N8", "B1", 10, 11, &["work", "carry"]);
	set_store(&mut world, c, 100);
	let json = format!(r#"[{{"o":"{:024x}","n":"build","d":{{"id":"{:024x}"}}}}]"#, c, site_id);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(world.rooms["W5N8"].objects[&site_id].progress, 0, "同格 creep 挡住时不施工");
}

/// issue#1 §4.7: 拆除工地掉落 floor(progress/2) 能量
#[test]
fn remove_construction_site_refunds_energy() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let site_id = alloc_id(&mut world.next_id);
	let mut site = objects::construction_site(site_id, "extension", 10, 10, 3000);
	site.progress = 1000;
	world.rooms.get_mut("W5N8").unwrap().objects.insert(site_id, site);
	let json = format!(
		r#"[{{"o":"room","n":"removeConstructionSite","d":{{"room":"W5N8","id":"{:024x}"}}}}]"#,
		site_id
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&site_id));
	let drop = room
		.objects
		.values()
		.find(|o| o.obj_type == "energy")
		.expect("应掉落能量");
	// 500 在当 tick 被衰减 1 → 499
	assert_eq!(drop.store.get("energy").copied().unwrap_or(0), 499, "floor(progress/2) 掉落后当 tick 衰减");
}

/// issue#1 §4.12: spawn.renewCreep 恢复寿命并扣能
#[test]
fn renew_creep_extends_life_and_charges_energy() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	let c = add_creep_in(&mut world, "W5N8", "R1", 25, 26, &["move", "carry", "work"]);
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&c).unwrap().age_time = Some(100);
	let before = store_of(&world, spawn);
	let json = format!(r#"[{{"o":"{:024x}","n":"renewCreep","d":{{"id":"{:024x}"}}}}]"#, spawn, c);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert_eq!(room.objects[&c].age_time, Some(300), "寿命 +floor(1.2*1500/3/3)=+200");
	// 扣 ceil(1.2*200/3/3)=27 能量; 同 tick spawn 自动回能 +1 (官方 spawns/tick.js) → 净 -26
	assert_eq!(before - store_of(&world, spawn), 26, "renew 扣 27 能量, spawn 同 tick 自动回能 +1");
}

/// issue#1 §4.12: spawn.recycleCreep 100% 回收 (携带 + 身体)
#[test]
fn recycle_creep_drops_full_resources() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	let c = add_creep_in(&mut world, "W5N8", "C1", 25, 26, &["carry", "move"]);
	set_store(&mut world, c, 50);
	let json = format!(r#"[{{"o":"{:024x}","n":"recycleCreep","d":{{"id":"{:024x}"}}}}]"#, spawn, c);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&c), "回收后单位应消失");
	let tomb = room
		.objects
		.values()
		.find(|o| o.obj_type == "tombstone")
		.expect("应留下墓碑");
	// 携带 50 + 身体 2×min(125,50×1.0)=100
	assert_eq!(tomb.store.get("energy").copied().unwrap_or(0), 150);
}

/// issue#1 §4.12: tower.repair 按官方功率修血并扣 10 能
#[test]
fn tower_repair_spends_energy() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tower = add_structure(&mut world, "W5N8", "tower", 20, 20);
	set_store(&mut world, tower, 100);
	let wall = add_structure(&mut world, "W5N8", "constructedWall", 20, 22);
	{
		let w = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&wall).unwrap();
		w.hits = 1000;
		w.hits_max = constants::WALL_HITS_MAX;
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"repair","d":{{"id":"{:024x}"}}}}]"#, tower, wall);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert_eq!(store_of(&world, tower), 90, "tower 每次消耗 10 能量");
	assert_eq!(room.objects[&wall].hits, 1800, "range 2 无衰减, 修 800");
}

/// issue#1 §4.12: link.transferEnergy 带 3% 损耗与冷却
#[test]
fn link_transfer_energy_with_loss_and_cooldown() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_structure(&mut world, "W5N8", "link", 10, 10);
	let b = add_structure(&mut world, "W5N8", "link", 12, 10);
	set_store(&mut world, a, 500);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"transferEnergy","d":{{"id":"{:024x}","amount":200}}}}]"#,
		a, b
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert_eq!(store_of(&world, a), 300, "源 link 扣 200");
	assert_eq!(store_of(&world, b), 194, "目标 link 收 200 - ceil(200*0.03)=194");
	assert_eq!(room.objects[&a].cooldown, 1, "冷却 = 1×距离2 再经当 tick 减 1");
}
