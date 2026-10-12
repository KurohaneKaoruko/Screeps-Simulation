//! 机制回归测试: 用受控意图直接驱动结算引擎, 锁定与官方规则对齐的关键数值。
//! 不经过 V8 —— 快且完全确定性; JS 侧 (PathFinder 跨房 / moveTo) 另见 shim_apis.rs。

use screeps_sim::engine::constants;
use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::objects;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::alloc_id;
use screeps_sim::engine::world::World;

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
	use screeps_sim::engine::objects::{creep_obj, BodyPart};
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
	let pile = screeps_sim::engine::objects::dropped_energy(id, 20, 20, 1000);
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
	assert_eq!(world.rooms["W5N8"].objects[&ctrl_id].safe_mode_available, 1, "官方: 升级时 safeModeAvailable +1");

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
	assert_eq!(controller.safe_mode_available, 0, "降级清空 safeModeAvailable");
	assert!(controller.safe_mode_cooldown.map(|t| t > world.tick).unwrap_or(false), "降级进入安全模式冷却");
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

/// GCL: 累计分数 gcl, 等级 floor((gcl/1e6)^(1/2.4))+1, 本级总量 L^2.4·1e6 − (L−1)^2.4·1e6 (官方 game.js:130-162)
#[test]
fn gcl_progress_total_matches_official() {
	assert_eq!(constants::gcl_progress_total(1), 1_000_000, "GCL1→2 = 1e6");
	assert_eq!(constants::gcl_progress_total(2), 4_278_031, "GCL2→3 = 2^2.4·1e6 − 1e6");
	assert_eq!(constants::gcl_level_for(0), 1);
	assert_eq!(constants::gcl_level_for(999_999), 1);
	assert_eq!(constants::gcl_level_for(1_000_000), 2);
	assert_eq!(constants::gcl_level_for(5_278_030), 2);
	assert_eq!(constants::gcl_level_for(5_278_031), 3, "到 GCL3 累计 2^2.4·1e6");
	assert_eq!(constants::gcl_level_for(14_000_000), 4, "3^2.4·1e6 ≈ 13.97e6");
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
	use screeps_sim::engine::objects::{creep_obj, BodyPart};
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
	// RCL5 才有 2 个 link 名额 (官方 links/transfer.js 检查 checkStructureAgainstController)
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().level = 5;
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

// ═════════════════════════ 战斗链路 (官方 processor/intents/creeps/*) ═════════════════════════

/// 把 (W5N8 房内) 对象标记为敌方: mine=false + enemy=true
fn mark_enemy(world: &mut World, id: u64) {
	let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&id).unwrap();
	o.mine = false;
	o.enemy = true;
}

fn hits_of(world: &World, id: u64) -> u32 {
	world.rooms["W5N8"].objects[&id].hits
}

/// 近战攻击: 3 ATTACK × 30 = 90 伤害, tick 末统一扣血; 目标反击 (hit-back) 30
#[test]
fn melee_attack_and_hit_back() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "MA", 20, 20, &["attack", "attack", "attack", "move"]);
	let b = add_creep(&mut world, "MB", 21, 20, &["attack", "move"]);
	let before_b = hits_of(&world, b);
	let before_a = hits_of(&world, a);
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, a, b);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(
		hits_of(&world, b),
		before_b - 90,
		"3 ATTACK × ATTACK_POWER(30) = 90, tick 末应用"
	);
	assert_eq!(
		hits_of(&world, a),
		before_a - 30,
		"被 1×ATTACK 目标反击 30 (hit-back)"
	);
}

/// 无 ATTACK 部件的目标不反击
#[test]
fn melee_attack_no_hit_back_without_attack_parts() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "NA", 20, 20, &["attack", "move"]);
	let b = add_creep(&mut world, "NB", 21, 20, &["carry", "move"]);
	let before_a = hits_of(&world, a);
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, a, b);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, a), before_a, "无 ATTACK 部件目标不反击");
}

/// 近战攻击 rampart 保护下的目标: 伤害重定向到 rampart
#[test]
fn melee_attack_redirects_to_rampart() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "RA", 20, 20, &["attack", "attack", "move"]);
	let b = add_creep(&mut world, "RB", 21, 20, &["attack", "move"]);
	let rampart = add_structure(&mut world, "W5N8", "rampart", 21, 20);
	{
		let r = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&rampart).unwrap();
		r.hits = 3000;
		r.hits_max = 3000;
		r.next_decay_time = Some(world.tick + 100); // 避开当 tick 衰减
	}
	let before_b = hits_of(&world, b);
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, a, b);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, b), before_b, "rampart 下的目标不掉血");
	assert_eq!(hits_of(&world, rampart), 3000 - 60, "伤害 60 重定向到 rampart");
}

/// 远程攻击: 3 格内全额 RANGED_ATTACK_POWER(10)/部件; 4 格外意图不生效
#[test]
fn ranged_attack_range_limit() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "RRA", 20, 20, &["ranged_attack", "ranged_attack", "move"]);
	let b = add_creep(&mut world, "RRB", 23, 20, &["move"]);
	let before = hits_of(&world, b);
	let json = format!(r#"[{{"o":"{:024x}","n":"rangedAttack","d":{{"id":"{:024x}"}}}}]"#, a, b);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, b), before - 20, "3 格内 2 部件 × 10 = 20");
	// 移到 4 格外
	{
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&b).unwrap();
		o.x = 24;
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"rangedAttack","d":{{"id":"{:024x}"}}}}]"#, a, b);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, b), before - 20, "4 格外不应造成伤害");
}

/// rangedMassAttack 距离衰减: 官方 {0:1, 1:1, 2:0.4, 3:0.1}
#[test]
fn ranged_mass_attack_distance_falloff() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "RMA", 20, 20, &["ranged_attack", "ranged_attack", "move"]);
	// 2 部件 × 10 × 1.0 = 20 基准功率
	let d1 = add_creep(&mut world, "D1", 21, 20, &["move"]);
	let d2 = add_creep(&mut world, "D2", 22, 20, &["move"]);
	let d3 = add_creep(&mut world, "D3", 23, 20, &["move"]);
	mark_enemy(&mut world, d1);
	mark_enemy(&mut world, d2);
	mark_enemy(&mut world, d3);
	let (b1, b2, b3) = (hits_of(&world, d1), hits_of(&world, d2), hits_of(&world, d3));
	let json = format!(r#"[{{"o":"{:024x}","n":"rangedMassAttack","d":{{}}}}]"#, a);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, d1), b1 - 20, "距离 1: 20 × 1.0");
	assert_eq!(hits_of(&world, d2), b2 - 8, "距离 2: 20 × 0.4 = 8");
	assert_eq!(hits_of(&world, d3), b3 - 2, "距离 3: 20 × 0.1 = 2");
}

/// 治疗: 近战 12/部件; rangedHeal 4/部件且可超过当前血量上限裁剪
#[test]
fn heal_creep_restores_hits() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let h = add_creep(&mut world, "H", 20, 20, &["heal", "heal", "move"]);
	let t = add_creep(&mut world, "T", 21, 20, &["move"]);
	let max = hits_of(&world, t);
	// 目标先受伤
	{
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&t).unwrap();
		o.hits = max - 30;
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"heal","d":{{"id":"{:024x}"}}}}]"#, h, t);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, t), max - 30 + 24, "2 HEAL × 12 = 24");
	// rangedHeal (距离 1 也算 in-range): 2 × 4 = 8, 但只差 6 → 回满
	let json = format!(r#"[{{"o":"{:024x}","n":"rangedHeal","d":{{"id":"{:024x}"}}}}]"#, h, t);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, t), max, "rangedHeal 溢出部分被裁剪");
}

/// 部件从头部开始损坏 (官方 _recalc-body: 血量从身体末尾倒序分配,
/// 因此 TOUGH 放最前面才是肉盾)
#[test]
fn damage_recalc_body_from_tail() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	// 身体: tough, attack, attack, move, move (hits_max = 500)
	let a = add_creep(&mut world, "BA", 20, 20, &["tough", "attack", "attack", "move", "move"]);
	let enemy_body: Vec<&str> = ["attack"; 9].iter().copied().chain(["move"]).collect();
	let enemy = add_creep(&mut world, "BE", 21, 20, &enemy_body);
	mark_enemy(&mut world, enemy);
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, enemy, a);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let obj = &world.rooms["W5N8"].objects[&a];
	assert_eq!(obj.hits, 230, "500 - 9×30 = 230 (tough/move 无 ATTACK 不反击)");
	assert_eq!(obj.body[0].hits, 0, "头部 tough 最先死");
	assert_eq!(obj.body[1].hits, 0, "第 2 个 attack 也死");
	assert_eq!(obj.body[2].hits, 30, "第 3 个 attack 剩 30");
	assert_eq!(obj.body[3].hits, 100, "尾部部件满血");
	assert_eq!(obj.body[4].hits, 100, "尾部部件满血");
}

/// dismantle: WORK 部件 50/个, 返还 floor(amount × 0.005) 能量 (官方 DISMANTLE_COST)
#[test]
fn dismantle_structure_refunds_half_energy() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let w = add_creep(&mut world, "DW", 20, 20, &["work", "work", "work", "work", "carry", "move"]);
	let wall = add_structure(&mut world, "W5N8", "constructedWall", 21, 20);
	{
		let s = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&wall).unwrap();
		s.hits = 1000;
		s.hits_max = constants::WALL_HITS_MAX;
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"dismantle","d":{{"id":"{:024x}"}}}}]"#, w, wall);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, wall), 1000 - 200, "4 WORK × DISMANTLE_POWER(50) = 200");
	assert_eq!(store_of(&world, w), 1, "返还 floor(200 × DISMANTLE_COST 0.005) = 1 能量");
}

/// 塔攻击 creep: 伤害累积到 tick 末应用; 塔能量扣 10
#[test]
fn tower_attack_creep_damage_and_cost() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tower = add_structure(&mut world, "W5N8", "tower", 20, 20);
	set_store(&mut world, tower, 100);
	// 10 个部件 (hits 1000): 600 伤害不足以致死
	let target_body: Vec<&str> = ["tough"; 9].iter().copied().chain(["move"]).collect();
	let target = add_creep(&mut world, "TT", 22, 20, &target_body);
	mark_enemy(&mut world, target);
	let before = hits_of(&world, target);
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, tower, target);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, tower), 90, "塔每次消耗 10 能量");
	assert_eq!(
		hits_of(&world, target),
		before - 600,
		"range 2 无衰减 TOWER_POWER_ATTACK=600"
	);
}

/// 塔攻击结构: 即时扣血; 血量归零销毁并掉落仓储 (掉落当 tick 衰减 ⌈200/1000⌉=1)
#[test]
fn tower_attack_destroys_structure_and_drops_store() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tower = add_structure(&mut world, "W5N8", "tower", 20, 20);
	set_store(&mut world, tower, 100);
	let ext = add_structure(&mut world, "W5N8", "extension", 22, 20);
	mark_enemy(&mut world, ext);
	{
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&ext).unwrap();
		o.hits = 500;
		o.store.insert("energy".into(), 200);
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#, tower, ext);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&ext), "600 > 500 应销毁");
	// 官方 _damage.js → structures/_destroy.js: 仓储留在 ruin 里 (不直接落地)
	assert!(!room.objects.values().any(|o| o.obj_type == "energy"), "不再直接掉落");
	let ruin = room.objects.values().find(|o| o.obj_type == "ruin").expect("应留下 ruin");
	assert_eq!(ruin.store.get("energy").copied(), Some(200));
	assert_eq!((ruin.x, ruin.y), (22, 20));
	assert_eq!(ruin.next_decay_time, Some(constants::RUIN_DECAY), "decayTime = 摧毁 tick(0) + 500");
	assert!(room.events.iter().any(|e| e.event == screeps_sim::engine::intents::EVENT_OBJECT_DESTROYED && e.object_id == ext));
}

/// claimController: 中立控制器 → 归属我方 level 1
#[test]
fn claim_controller_takes_ownership() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let claimer = add_creep(&mut world, "CL", 27, 26, &["claim", "move"]);
	let ctrl = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap()
		.id;
	{
		let c = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&ctrl).unwrap();
		c.mine = false;
		c.level = 0;
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"claimController","d":{{"id":"{:024x}"}}}}]"#, claimer, ctrl);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert!(c.mine, "claim 后归属我方");
	assert_eq!(c.level, 1, "claim 后 level 1");
}

/// reserveController: 官方 CONTROLLER_RESERVE=1 (每 CLAIM 部件每 tick +1), 上限 5000
#[test]
fn reserve_controller_caps_at_max() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let claimer = add_creep(&mut world, "RS", 27, 26, &["claim", "move"]);
	let ctrl = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap()
		.id;
	{
		let c = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&ctrl).unwrap();
		c.mine = false;
		c.level = 0;
		c.reservation_end_time = Some(world.tick + 4999); // 只差 1
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"reserveController","d":{{"id":"{:024x}"}}}}]"#, claimer, ctrl);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(
		c.reservation_end_time,
		Some(4999 + 1),
		"1 CLAIM × CONTROLLER_RESERVE(1) = +1"
	);
	// 已达上限 (endTime + 1 > now + 5000): 不再增长
	let capped_at = world.tick + constants::CONTROLLER_RESERVE_MAX;
	{
		let c = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&ctrl).unwrap();
		c.reservation_end_time = Some(capped_at);
	}
	let json = format!(r#"[{{"o":"{:024x}","n":"reserveController","d":{{"id":"{:024x}"}}}}]"#, claimer, ctrl);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(
		world.rooms["W5N8"].objects[&ctrl].reservation_end_time,
		Some(capped_at),
		"endTime + effect > now + CONTROLLER_RESERVE_MAX(5000) 时不生效"
	);
}

/// attackController: 有归属的控制器被攻击 → upgradeBlocked + 降级计时提前
#[test]
fn attack_controller_blocks_upgrade() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let claimer = add_creep(&mut world, "AC", 27, 26, &["claim", "move"]);
	let ctrl = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap()
		.id;
	let before_downgrade = world.rooms["W5N8"].objects[&ctrl].downgrade_time;
	let json = format!(
		r#"[{{"o":"{:024x}","n":"attackController","d":{{"id":"{:024x}"}}}}]"#,
		claimer, ctrl
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let c = &world.rooms["W5N8"].objects[&ctrl];
	assert_eq!(
		c.upgrade_blocked,
		world.tick - 1 + constants::CONTROLLER_ATTACK_BLOCKED_UPGRADE,
		"被攻击后 1000t 内无法升级 (基于意图执行时的 tick)"
	);
	let dt = c.downgrade_time.unwrap();
	assert_eq!(
		dt,
		before_downgrade.unwrap().saturating_sub(constants::CONTROLLER_CLAIM_DOWNGRADE),
		"降级计时提前 CONTROLLER_CLAIM_DOWNGRADE"
	);
}

/// signController: 签名写入控制器并可清除
#[test]
fn sign_controller_writes_and_clears() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let s = add_creep(&mut world, "SG", 27, 26, &["move"]);
	let ctrl = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "controller")
		.unwrap()
		.id;
	let json = format!(
		r#"[{{"o":"{:024x}","n":"signController","d":{{"id":"{:024x}","sign":"hello"}}}}]"#,
		s, ctrl
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(
		world.rooms["W5N8"].objects[&ctrl].sign.as_deref(),
		Some("hello")
	);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"signController","d":{{"id":"{:024x}","sign":""}}}}]"#,
		s, ctrl
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(world.rooms["W5N8"].objects[&ctrl].sign, None, "空签名清除");
}

/// 官方 checkPriorities 互斥: attack 与 harvest 同 tick 提交 → harvest 被压制
#[test]
fn attack_suppresses_harvest_same_tick() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_creep(&mut world, "PR", 20, 20, &["attack", "work", "move"]);
	let enemy = add_creep(&mut world, "PE", 21, 20, &["move"]);
	mark_enemy(&mut world, enemy);
	let source = first_source(&world);
	let sx = world.rooms["W5N8"].objects[&source].x;
	let sy = world.rooms["W5N8"].objects[&source].y;
	{
		// 把攻击者移到 source 旁边
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&a).unwrap();
		o.x = sx;
		o.y = sy + 1;
		// 敌人也得在旁边
		let e = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&enemy).unwrap();
		e.x = sx;
		e.y = sy;
	}
	let before = hits_of(&world, enemy);
	let source_before = world.rooms["W5N8"].objects[&source]
		.store
		.get("energy")
		.copied()
		.unwrap_or(0);
	let json = format!(
		r#"[{{"o":"{:024x}","n":"harvest","d":{{"id":"{:024x}"}}}},{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#,
		a, source, a, enemy
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, enemy), before - 30, "attack 正常执行");
	assert_eq!(
		world.rooms["W5N8"].objects[&source]
			.store
			.get("energy")
			.copied()
			.unwrap_or(0),
		source_before,
		"同 tick 的 harvest 被互斥表压制"
	);
}

/// 战斗死亡: creep 血量归零 → 移除 + 墓碑 + creeps_lost 统计
#[test]
fn combat_death_creates_tombstone() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let victim = add_creep(&mut world, "VD", 21, 20, &["tough", "move"]);
	let killer_body: Vec<&str> = ["attack"; 9].iter().copied().chain(["move"]).collect();
	let killer = add_creep(&mut world, "VK", 20, 20, &killer_body);
	let before_lost = world.creeps_lost;
	let json = format!(
		r#"[{{"o":"{:024x}","n":"attack","d":{{"id":"{:024x}"}}}}]"#,
		killer, victim
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&victim), "270 > tough+move 200 应击杀");
	assert_eq!(world.creeps_lost, before_lost + 1, "击杀计数 +1");
	assert!(
		room.objects.values().any(|o| o.obj_type == "tombstone"),
		"战斗死亡留墓碑"
	);
}

/// issue#1 §五: 孵化扣能顺序 — 先 spawn (就近优先), 再 extension (就近优先),
/// 官方 _charge-energy 语义。RCL2 场景: spawn(30) + 近 ext(50) + 远 ext(50),
/// body 成本 90 (attack 80 + tough 10) → spawn 清空、近 ext 清空、远 ext 剩 40。
#[test]
fn spawn_creep_charges_spawn_then_nearest_extensions() {
	let scene = r#"{
		"name": "charge",
		"seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "controller": { "level": 2, "pos": [28, 27] },
			  "structures": [
			    { "type": "spawn", "pos": [25, 25], "energy": 30 },
			    { "type": "extension", "pos": [26, 25], "energy": 50 },
			    { "type": "extension", "pos": [40, 40], "energy": 50 }
			  ] }
		]
	}"#;
	let (mut world, mut movement) = setup(scene);
	let spawn_id = world.rooms["W5N8"]
		.objects
		.values()
		.find(|o| o.obj_type == "spawn")
		.unwrap()
		.id;
	let energy_at = |world: &World, x: u8, y: u8| -> u32 {
		world.rooms["W5N8"]
			.objects
			.values()
			.find(|o| o.x == x && o.y == y && o.obj_type != "creep")
			.map(|o| o.store.get("energy").copied().unwrap_or(0))
			.unwrap_or(0)
	};
	let json = format!(
		r#"[{{"o":"{:024x}","n":"spawnCreep","d":{{"body":["attack","tough"],"name":"B1"}}}}]"#,
		spawn_id
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	// 孵化扣能后房间 energyAvailable=40 < 300 → 同 tick 触发 spawn 自动回能 +1 (官方 spawns/tick)
	assert_eq!(energy_at(&world, 25, 25), 1, "spawn 应最先被扣空 (30 全部), 随后回能 +1");
	assert_eq!(energy_at(&world, 26, 25), 0, "近 extension 其次 (50 全部)");
	assert_eq!(energy_at(&world, 40, 40), 40, "远 extension 只补差额 10, 剩 40");
	assert!(
		world.rooms["W5N8"]
			.objects
			.values()
			.any(|o| o.obj_type == "creep" && o.name.as_deref() == Some("B1") && o.spawning),
		"孵化中的 B1 应已落位 (spawning)"
	);
}

/// 直接序列化器 (build_snapshot → String) 与 Value 版 (build_snapshot_value)
/// 必须语义一致: 直接序列化器是热路径, Value 版是对拍基准。任何分歧都意味着
/// shim 读到的世界不同 —— 字段增删时两处同步修改并跑此测试。
#[test]
fn snapshot_direct_serializer_matches_value_tree() {
	use screeps_sim::engine::snapshot::{build_snapshot, build_snapshot_value, CpuView, SnapshotOptions};
	let scenario = Scenario::from_json(
		r#"{
		"name": "parity",
		"seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "sources": { "positions": [[10, 10]] },
			  "mineral": { "pos": [40, 40], "mineral_type": "Z", "amount": 2500 },
			  "controller": { "level": 3, "pos": [28, 27] },
			  "structures": [
			    { "type": "spawn", "pos": [25, 25], "energy": 200 },
			    { "type": "extension", "pos": [26, 25], "energy": 50 },
			    { "type": "tower", "pos": [27, 25], "energy": 300 },
			    { "type": "link", "pos": [28, 25], "energy": 100 },
			    { "type": "storage", "pos": [24, 25], "energy": 4000 },
			    { "type": "container", "pos": [23, 25], "energy": 900 },
			    { "type": "road", "pos": [22, 25] },
			    { "type": "rampart", "pos": [25, 26] },
			    { "type": "constructedWall", "pos": [25, 27] }
			  ],
			  "creeps": [{ "name": "w\"q", "body": ["tough", "work", "carry", "move"], "pos": [10, 11],
			               "store": { "energy": 42 } }] }
		]
	}"#,
	)
	.unwrap();
	let mut world = build_world(&scenario).unwrap();

	// 富化状态: 掉落能量 / 墓碑 / 控制器签名与保留 / 容器衰减
	{
		let room = world.rooms.values_mut().next().unwrap();
		let mut next = world.next_id;
		let id1 = alloc_id(&mut next);
		let id2 = alloc_id(&mut next);
		let id3 = alloc_id(&mut next);
		world.next_id = next;
		room.objects.insert(id1, objects::dropped_energy(id1, 15, 15, 350));
		room.objects.insert(id2, objects::tombstone(id2, 16, 16, "w\"q", world.tick + 300));
		room.objects
			.insert(id3, objects::construction_site(id3, "road", 20, 20, 300));
		// 废墟: 有主 (tower) 与无主 (container) 两种 structure 形态
		let mut next = world.next_id;
		let (r1, r2) = (alloc_id(&mut next), alloc_id(&mut next));
		world.next_id = next;
		let mut tower = objects::structure(9001, "tower", 30, 30, true);
		tower.store.insert("energy".into(), 70);
		room.objects.insert(r1, objects::ruin(r1, &tower, 0));
		let cont = objects::structure(9002, "container", 31, 31, false);
		room.objects.insert(r2, objects::ruin(r2, &cont, 0));
		if let Some(c) = room.controller_mut() {
			c.sign = Some("签名 \"引号\" & <tag>".to_string());
			c.reservation_end_time = Some(world.tick + 500);
		}
		for o in room.objects.values_mut() {
			if o.obj_type == "container" {
				o.next_decay_time = Some(world.tick + 400);
			}
		}
	}
	// 跑一个意图 tick: 采集/孵化/建造路径都会改变字段形态
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
	let spawn_id = world
		.rooms
		.values()
		.next()
		.unwrap()
		.objects
		.values()
		.find(|o| o.obj_type == "spawn")
		.map(|o| format!("{:024x}", o.id))
		.unwrap();
	let intents = IntentList::parse(&format!(
		r#"[{{"o":"{creep_id}","n":"harvest"}},{{"o":"{spawn_id}","n":"spawnCreep","d":{{"body":["work","move"],"name":"b1","directions":[3,5]}}}}]"#
	));
	let mut movement = Movement::default();
	tick::run_tick(&mut world, &mut movement, &intents);

	for include_terrain in [true, false] {
		let opts = SnapshotOptions {
			cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10_000.0 },
			always_visible: true,
			include_terrain,
		};
		let direct = build_snapshot(&world, &opts);
		let parsed: serde_json::Value =
			serde_json::from_str(&direct).expect("直接序列化必须是合法 JSON");
		let reference = build_snapshot_value(&world, &opts);
		assert_eq!(
			parsed, reference,
			"直接序列化器与 Value 版输出不一致 (include_terrain={include_terrain})"
		);
		if !include_terrain {
			assert_eq!(parsed["terrain"], serde_json::json!({}), "不含 terrain 时应为空对象");
		}
	}
}

/// pull: 无 MOVE 的矿工被拖车拉着走, 矿工的移动疲劳记到拖车身上 (官方 _add-fatigue 沿 _pulled 链)。
/// 拖车 2 MOVE、矿工 5 WORK: 每步给拖车加 5×2=10 疲劳, 拖车每 tick 回 4 → 每 3 tick 走一步。
#[test]
fn pull_moves_creep_without_move_parts() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tug = add_creep(&mut world, "Tug", 25, 20, &["move", "move"]);
	let miner = add_creep(&mut world, "Miner", 24, 20, &["work", "work", "work", "work", "work"]);
	let json = format!(
		r#"[{{"o":"{tug:024x}","n":"move","d":{{"direction":3}}}},
		    {{"o":"{tug:024x}","n":"pull","d":{{"id":"{miner:024x}"}}}},
		    {{"o":"{miner:024x}","n":"move","d":{{"id":"{tug:024x}"}}}}]"#
	);
	let mut trail = Vec::new();
	for _ in 0..7 {
		tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
		let (_, tx, _) = find_creep(&world, "Tug");
		let (_, mx, _) = find_creep(&world, "Miner");
		let tf = world.rooms["W5N8"].objects[&tug].fatigue;
		let mf = world.rooms["W5N8"].objects[&miner].fatigue;
		trail.push((tx, mx, tf, mf));
	}
	assert_eq!(trail[0], (26, 25, 6, 0), "第 1 tick: 两者各进一格, 矿工的 10 疲劳记给拖车再回 4: {trail:?}");
	assert_eq!(trail.last().map(|t| (t.0, t.1)), Some((28, 27)), "7 tick 走 3 步: {trail:?}");
	assert!(trail.iter().all(|t| t.3 == 0), "被拉者自身不积累疲劳: {trail:?}");
}

/// 没有 pull 时, 无 MOVE 的 creep 跟随 move(creep) 也不能动
#[test]
fn move_toward_creep_without_pull_needs_move_parts() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tug = add_creep(&mut world, "Tug", 25, 20, &["move", "move"]);
	let _miner = add_creep(&mut world, "Miner", 24, 20, &["work"]);
	let miner = find_creep(&world, "Miner").0;
	let json = format!(
		r#"[{{"o":"{tug:024x}","n":"move","d":{{"direction":3}}}},
		    {{"o":"{miner:024x}","n":"move","d":{{"id":"{tug:024x}"}}}}]"#
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(find_creep(&world, "Tug").1, 26);
	assert_eq!(find_creep(&world, "Miner").1, 24, "没人拉、没 MOVE → 原地");
}

// ═════════════════════════ RCL 停用 / spawn 回能 (官方 _calc_spawns / checkStructureAgainstController / spawns/tick) ═════════════════════════

/// 官方 _calc_spawns: 超限时按到控制器距离**升序**保留最近的 limit 个, 停用最远的。
/// RCL2 extension 上限 5, 放 6 个: 只有最远那个 off; RCL2 spawn 上限 1, 放 2 个: 远的那个 off。
#[test]
fn spawn_off_keeps_nearest_within_limit() {
	let json = r#"{
		"name": "t", "seed": 1,
		"rooms": [{
			"name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 2, "pos": [10, 10] },
			"structures": [
				{ "type": "spawn", "pos": [40, 40], "energy": 300 },
				{ "type": "spawn", "pos": [12, 12], "energy": 300 },
				{ "type": "extension", "pos": [45, 45], "energy": 50 },
				{ "type": "extension", "pos": [11, 12], "energy": 50 },
				{ "type": "extension", "pos": [12, 11], "energy": 50 },
				{ "type": "extension", "pos": [13, 13], "energy": 50 },
				{ "type": "extension", "pos": [14, 14], "energy": 50 },
				{ "type": "extension", "pos": [15, 15], "energy": 50 }
			]
		}]
	}"#;
	let (world, _) = setup(json);
	let room = &world.rooms["W5N8"];
	let off_at = |x: u8, y: u8| room.objects.values().find(|o| o.x == x && o.y == y).unwrap().off;
	assert!(!off_at(12, 12), "近的 spawn 保留");
	assert!(off_at(40, 40), "远的 spawn 停用");
	assert!(off_at(45, 45), "最远的 extension 停用");
	for (x, y) in [(11, 12), (12, 11), (13, 13), (14, 14), (15, 15)] {
		assert!(!off_at(x, y), "近的 5 个 extension 保留 ({x},{y})");
	}
	assert_eq!(room.energy_available(), 300 + 5 * 50, "停用结构不计入 energyAvailable");
	// checkStructureAgainstController 与 off 结论一致
	for o in room.objects.values().filter(|o| o.obj_type == "spawn" || o.obj_type == "extension") {
		assert_eq!(room.structure_active(o), !o.off, "isActive 与 off 应一致: {:?}", (o.obj_type.as_str(), o.x, o.y));
	}
}

/// 官方 checkStructureAgainstController 适用于所有受 CONTROLLER_STRUCTURES 约束的类型:
/// RCL3 塔上限 1 → 两座塔只有离控制器近的可用; 同距离时迭代序 (id) 在前者优先; 路/容器无主恒可用
#[test]
fn structure_active_by_distance_for_towers() {
	let (mut world, _) = setup(RCL3_ROOM);
	let far = add_structure(&mut world, "W5N8", "tower", 40, 40);
	let near = add_structure(&mut world, "W5N8", "tower", 30, 27);
	let road = alloc_id(&mut world.next_id);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(road, objects::structure(road, "road", 5, 5, false));
	let room = &world.rooms["W5N8"];
	assert!(room.structure_active(&room.objects[&near]), "近塔可用");
	assert!(!room.structure_active(&room.objects[&far]), "远塔超出 RCL3 上限");
	assert!(room.structure_active(&room.objects[&road]), "无主结构恒可用");
	// 同距离: id 小的优先
	let (mut world, _) = setup(RCL3_ROOM);
	let a = add_structure(&mut world, "W5N8", "tower", 30, 27);
	let b = add_structure(&mut world, "W5N8", "tower", 26, 27);
	let room = &world.rooms["W5N8"];
	assert!(room.structure_active(&room.objects[&a]));
	assert!(!room.structure_active(&room.objects[&b]));
	// 控制器降到 0 级: 全部不可用
	let (mut world, _) = setup(RCL3_ROOM);
	let t = add_structure(&mut world, "W5N8", "tower", 30, 27);
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().level = 0;
	let room = &world.rooms["W5N8"];
	assert!(!room.structure_active(&room.objects[&t]));
}

/// 官方 processor.js:326: energyAvailable 在逐对象 tick 前只算一次, 所有 spawn 共用。
/// 两个 spawn 合计 299 → 两个都各回 1 (旧实现逐 spawn 重算, 第二个看到 300 就不回)
#[test]
fn spawn_regen_uses_room_energy_computed_once() {
	let json = r#"{
		"name": "t", "seed": 1,
		"rooms": [{
			"name": "W5N8", "terrain": { "kind": "flat" },
			"controller": { "level": 7, "pos": [28, 27] },
			"structures": [
				{ "type": "spawn", "pos": [25, 25], "energy": 100 },
				{ "type": "spawn", "pos": [20, 20], "energy": 199 }
			]
		}]
	}"#;
	let (mut world, mut movement) = setup(json);
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	let room = &world.rooms["W5N8"];
	let e = |x: u8| room.objects.values().find(|o| o.obj_type == "spawn" && o.x == x).unwrap().store["energy"];
	assert_eq!((e(25), e(20)), (101, 200), "两个 spawn 都应各回 1 能量");
	assert_eq!(room.energy_available(), 301);
}

/// 官方 spawns/tick.js: 控制器未归属 (level 0) 时 spawn 不回能
#[test]
fn spawn_regen_requires_owned_controller() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	set_store(&mut world, spawn, 100);
	{
		let c = world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap();
		c.level = 0;
		c.mine = false;
	}
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(store_of(&world, spawn), 100, "控制器无主 → 不回能");
}

// ═════════════════════════ 孵化: Spawning 意图 / spawnCreep 校验 / 出生落位 ═════════════════════════

fn spawn_intent(spawn: u64, name: &str, data: &str) -> IntentList {
	IntentList::parse(&format!(r#"[{{"o":"{spawn:024x}","n":"{name}","d":{data}}}]"#))
}

fn creep_named<'a>(world: &'a World, name: &str) -> Option<&'a objects::Obj> {
	world.rooms["W5N8"].objects.values().find(|o| o.obj_type == "creep" && o.name.as_deref() == Some(name))
}

/// 官方 cancelSpawning: 移除孵化中的 creep 并清空 spawning (能量不退)
#[test]
fn cancel_spawning_removes_creep() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["work","carry","move"],"name":"K"}"#));
	assert!(creep_named(&world, "K").map(|c| c.spawning).unwrap_or(false));
	let energy_after_spawn = store_of(&world, spawn);
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "cancelSpawning", "{}"));
	assert!(creep_named(&world, "K").is_none(), "取消后孵化中的 creep 消失");
	assert!(world.rooms["W5N8"].objects[&spawn].spawn_job.is_none(), "spawning 清空");
	assert_eq!(store_of(&world, spawn), energy_after_spawn + 1, "不退能量 (只有自然回能 +1)");
}

/// 官方 setSpawnDirections: 合法方向覆盖出生方向序; 非法 (越界/非整数/空) 整体忽略
#[test]
fn set_spawn_directions_validates_and_applies() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move","move","move"],"name":"D"}"#));
	let dirs = |w: &World| w.rooms["W5N8"].objects[&spawn].spawn_job.as_ref().unwrap().directions.clone();
	assert_eq!(dirs(&world), None);
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "setSpawnDirections", r#"{"directions":[9,1]}"#));
	assert_eq!(dirs(&world), None, "含越界方向 → 忽略");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "setSpawnDirections", r#"{"directions":[1.5]}"#));
	assert_eq!(dirs(&world), None, "非整数 → 忽略");
	// 1 部件孵化 3 tick: 第 2/3 tick 之间设好方向 5,5,"3" → 去重为 [5,3]
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move","move"],"name":"D"}"#));
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "setSpawnDirections", r#"{"directions":[5,5,"3"]}"#));
	assert_eq!(dirs(&world), Some(vec![5, 3]));
	for _ in 0..6 {
		tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	}
	let c = creep_named(&world, "D").unwrap();
	assert!(!c.spawning);
	assert_eq!((c.x, c.y), (25, 26), "按方向 5 (BOTTOM) 出生");
}

/// 官方 create-creep.js: directions 非法 / spawn 停用 / 非我方 → 不孵化
#[test]
fn spawn_creep_rejects_invalid_directions_and_inactive_spawn() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move"],"name":"X","directions":[0]}"#));
	assert!(creep_named(&world, "X").is_none(), "方向 0 非法 → 意图作废");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move"],"name":"X","directions":"5"}"#));
	assert!(creep_named(&world, "X").is_none(), "directions 不是数组 → 意图作废");
	// 控制器降为 0 级: spawn 不可用 (checkStructureAgainstController)
	{
		let c = world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap();
		c.level = 0;
	}
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move"],"name":"Y"}"#));
	assert!(creep_named(&world, "Y").is_none(), "RCL0 spawn 停用 → 不孵化");
	// 非我方 spawn
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&spawn).unwrap().mine = false;
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move"],"name":"Z"}"#));
	assert!(creep_named(&world, "Z").is_none(), "非我方 spawn 不孵化");
}

/// energyStructures 重复 id 只算一次 (官方 _.uniq): 200 能量的 spawn 列两次不能付 300 的身体;
/// 旧实现会按 400 判定够用, 扣费时 u32 下溢
#[test]
fn spawn_energy_structures_dedup() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	set_store(&mut world, spawn, 200);
	let ids = format!(r#"["{spawn:024x}","{spawn:024x}"]"#);
	let data = format!(r#"{{"body":["work","work","move","move"],"name":"E","energyStructures":{ids}}}"#);
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", &data));
	assert!(creep_named(&world, "E").is_none(), "去重后只有 200 能量, 不够 300");
	assert_eq!(store_of(&world, spawn), 201, "未扣费, 仅自然回能");
	let data = format!(r#"{{"body":["work","move"],"name":"F","energyStructures":{ids}}}"#);
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", &data));
	assert!(creep_named(&world, "F").is_some(), "150 够用");
	assert_eq!(store_of(&world, spawn), 201 - 150 + 1);
	// 显式空列表 → 只用列出的结构 (没有) → 失败
	let data = r#"{"body":["move"],"name":"G","energyStructures":[]}"#;
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", data));
	assert!(creep_named(&world, "G").is_none(), "energyStructures=[] 无可用能量");
}

/// 出生落位: 越界方向不再被夹成 TOP; 墙格上有路时可出生 (官方 _born-creep.js)
#[test]
fn birth_tile_directions_and_road_on_wall() {
	use screeps_sim::engine::movement::find_birth_tile;
	let (mut world, _) = setup(TWO_ROOMS);
	let spawn = spawn_of(&world, "W5N8");
	let movement = Movement::default();
	{
		let room = &world.rooms["W5N8"];
		let s = &room.objects[&spawn];
		assert_eq!(find_birth_tile(room, s, &movement, Some(&[0, 9])), None, "越界方向不映射到 TOP");
		assert_eq!(find_birth_tile(room, s, &movement, Some(&[0, 3])), Some((26, 25)), "跳过非法值, 用方向 3");
	}
	// spawn (25,25) 周围全是墙, 只有 (26,26) 的墙上有路
	{
		let room = world.rooms.get_mut("W5N8").unwrap();
		for dx in -1i32..=1 {
			for dy in -1i32..=1 {
				if dx != 0 || dy != 0 {
					room.terrain.set((25 + dx) as usize, (25 + dy) as usize, 1);
				}
			}
		}
	}
	{
		let room = &world.rooms["W5N8"];
		assert_eq!(find_birth_tile(room, &room.objects[&spawn], &movement, None), None, "四面是墙无处出生");
	}
	let road = alloc_id(&mut world.next_id);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(road, objects::structure(road, "road", 26, 26, false));
	let room = &world.rooms["W5N8"];
	assert_eq!(find_birth_tile(room, &room.objects[&spawn], &movement, None), Some((26, 26)), "墙上路可出生");
}

// ═════════════════════════ tower / link 结算细节 ═════════════════════════

/// 官方 links/transfer.js: 超出 RCL 上限的源 link 不能发送 (RCL1 无 link 名额)
#[test]
fn link_transfer_requires_rcl() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let a = add_structure(&mut world, "W5N8", "link", 10, 10);
	let b = add_structure(&mut world, "W5N8", "link", 12, 10);
	set_store(&mut world, a, 500);
	let json = format!(r#"[{{"o":"{a:024x}","n":"transferEnergy","d":{{"id":"{b:024x}","amount":200}}}}]"#);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, a), 500, "RCL1 link 不可用, 不转移");
	assert_eq!(store_of(&world, b), 0);
	// RCL5: 目标剩 100 空间时, 处理器把 200 截到 100 (官方行为)
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().level = 5;
	set_store(&mut world, b, 700);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(store_of(&world, a), 400, "截到目标剩余容量 100");
	assert_eq!(store_of(&world, b), 797, "700 + 100 - ceil(3)");
}

/// 官方 towers/attack.js: 不攻击孵化中的 creep
#[test]
fn tower_does_not_attack_spawning_creep() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let tower = add_structure(&mut world, "W5N8", "tower", 20, 20);
	set_store(&mut world, tower, 100);
	let target = add_creep(&mut world, "Baby", 22, 20, &["move", "move", "move", "move", "move", "move", "move"]);
	mark_enemy(&mut world, target);
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&target).unwrap().spawning = true;
	let spawn = spawn_of(&world, "W5N8");
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&spawn).unwrap().spawn_job =
		Some(objects::SpawnJob { name: "Baby".into(), need_time: 21, spawn_time: 100, directions: None });
	let json = format!(r#"[{{"o":"{tower:024x}","n":"attack","d":{{"id":"{target:024x}"}}}}]"#);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert_eq!(hits_of(&world, target), 700, "孵化中不受伤");
	assert_eq!(store_of(&world, tower), 100, "攻击未执行不扣能");
}

// ═════════════════════════ destroyStructure / ruin (官方 room/destroy-structure.js, structures/_destroy.js, ruins/tick.js) ═════════════════════════

fn destroy_intent(id: u64) -> IntentList {
	IntentList::parse(&format!(r#"[{{"o":"room","n":"destroyStructure","d":{{"room":"W5N8","id":"{id:024x}"}}}}]"#))
}

/// destroyStructure: 留下继承仓储的 ruin, 500t 后消散并把资源散落原地; 不记 EVENT_OBJECT_DESTROYED
#[test]
fn destroy_structure_leaves_ruin_that_decays() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let tower = add_structure(&mut world, "W5N8", "tower", 20, 20);
	set_store(&mut world, tower, 400);
	tick::run_tick(&mut world, &mut movement, &destroy_intent(tower));
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&tower));
	assert!(room.events.iter().all(|e| e.event != screeps_sim::engine::intents::EVENT_OBJECT_DESTROYED));
	let ruin = room.objects.values().find(|o| o.obj_type == "ruin").expect("应留下 ruin");
	let ruin_id = ruin.id;
	let info = ruin.ruin.as_ref().unwrap();
	assert_eq!((info.structure_id, info.structure_type.as_str(), info.destroy_time), (tower, "tower", 0));
	assert_eq!(info.hits_max, 3000);
	assert_eq!(ruin.store.get("energy").copied(), Some(400));
	// decayTime = 0 + 500 → 第 499 tick 消散
	for _ in 1..499 {
		tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	}
	assert!(world.rooms["W5N8"].objects.contains_key(&ruin_id), "tick 498 仍在");
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	let room = &world.rooms["W5N8"];
	assert!(!room.objects.contains_key(&ruin_id), "tick 499 消散");
	let pile = room.objects.values().find(|o| o.obj_type == "energy" && o.x == 20 && o.y == 20).expect("资源落地");
	assert_eq!(pile.store.get("energy").copied(), Some(400), "ruin tick 中新落地的资源本 tick 不再衰减");
}

/// destroyStructure 的官方前置: 控制器非我方 / 房内有敌方 creep / 目标无建造成本 → 不执行
#[test]
fn destroy_structure_preconditions() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let ext = add_structure(&mut world, "W5N8", "extension", 20, 20);
	let foe = add_creep(&mut world, "Foe", 40, 40, &["move"]);
	mark_enemy(&mut world, foe);
	tick::run_tick(&mut world, &mut movement, &destroy_intent(ext));
	assert!(world.rooms["W5N8"].objects.contains_key(&ext), "有敌方 creep → 不摧毁");
	world.rooms.get_mut("W5N8").unwrap().objects.remove(&foe);
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().mine = false;
	tick::run_tick(&mut world, &mut movement, &destroy_intent(ext));
	assert!(world.rooms["W5N8"].objects.contains_key(&ext), "控制器非我方 → 不摧毁");
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().mine = true;
	let ctrl = world.rooms["W5N8"].controller().unwrap().id;
	tick::run_tick(&mut world, &mut movement, &destroy_intent(ctrl));
	assert!(world.rooms["W5N8"].objects.contains_key(&ctrl), "controller 无建造成本 → 不摧毁");
	let site = alloc_id(&mut world.next_id);
	world.rooms.get_mut("W5N8").unwrap().objects.insert(site, objects::construction_site(site, "road", 5, 5, 300));
	tick::run_tick(&mut world, &mut movement, &destroy_intent(site));
	assert!(world.rooms["W5N8"].objects.contains_key(&site), "工地走 removeConstructionSite, destroyStructure 无效");
	tick::run_tick(&mut world, &mut movement, &destroy_intent(ext));
	assert!(!world.rooms["W5N8"].objects.contains_key(&ext), "条件满足 → 摧毁");
}

/// 摧毁正在孵化的 spawn: 孵化中的 creep 一并移除 (官方 _destroy.js)
#[test]
fn destroy_spawning_spawn_removes_creep() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let spawn = spawn_of(&world, "W5N8");
	tick::run_tick(&mut world, &mut movement, &spawn_intent(spawn, "spawnCreep", r#"{"body":["move","move"],"name":"Kid"}"#));
	assert!(creep_named(&world, "Kid").is_some());
	tick::run_tick(&mut world, &mut movement, &destroy_intent(spawn));
	assert!(creep_named(&world, "Kid").is_none(), "孵化中的 creep 随 spawn 一起消失");
	assert!(world.rooms["W5N8"].objects.values().any(|o| o.obj_type == "ruin"));
}

// ═════════════════════════ rampart / container / road 细节 (官方 ramparts/tick.js, containers/tick.js) ═════════════════════════

/// rampart hitsMax 每 tick 刷新 (不等衰减到期); 与控制器不同归属 → 0
#[test]
fn rampart_hits_max_refreshes_every_tick() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let id = add_structure(&mut world, "W5N8", "rampart", 10, 10);
	{
		let o = world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&id).unwrap();
		o.hits = 5000;
		o.hits_max = 0;
		o.next_decay_time = Some(50);
	}
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(world.rooms["W5N8"].objects[&id].hits_max, constants::rampart_hits_max(3), "未到衰减点也刷新");
	assert_eq!(world.rooms["W5N8"].objects[&id].hits, 5000, "未到衰减点不掉血");
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().level = 5;
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(world.rooms["W5N8"].objects[&id].hits_max, constants::rampart_hits_max(5));
	mark_enemy(&mut world, id);
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(world.rooms["W5N8"].objects[&id].hits_max, 0, "敌方 rampart 在我方控制器房间 hitsMax=0");
}

/// rampart.setPublic 意图写入 isPublic 并持久
#[test]
fn rampart_set_public_persists() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let id = add_structure(&mut world, "W5N8", "rampart", 10, 10);
	world.rooms.get_mut("W5N8").unwrap().objects.get_mut(&id).unwrap().hits = 5000;
	let set = |v: &str| IntentList::parse(&format!(r#"[{{"o":"{id:024x}","n":"setPublic","d":{{"isPublic":{v}}}}}]"#));
	tick::run_tick(&mut world, &mut movement, &set("true"));
	assert!(world.rooms["W5N8"].objects[&id].is_public);
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert!(world.rooms["W5N8"].objects[&id].is_public, "跨 tick 保留");
	tick::run_tick(&mut world, &mut movement, &set("false"));
	assert!(!world.rooms["W5N8"].objects[&id].is_public);
}

/// 官方 containers/tick.js: 控制器 level > 0 (不论归属) → 衰减周期 500
#[test]
fn container_decay_interval_uses_controller_level_only() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	let id = add_structure(&mut world, "W5N8", "container", 10, 10);
	{
		let room = world.rooms.get_mut("W5N8").unwrap();
		let o = room.objects.get_mut(&id).unwrap();
		o.mine = false;
		o.next_decay_time = Some(0);
		room.controller_mut().unwrap().mine = false; // 别人的 3 级控制器
	}
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(world.rooms["W5N8"].objects[&id].next_decay_time, Some(500));
}

/// 保留: 可以连续续保留; 不再续则到期清除 (官方 controllers/tick.js:10);
/// 被保留期间 source 容量 3000, 到期回中立 1500 (官方 sources/tick.js); 无控制器的房间 4000 (SK 房)
#[test]
fn reservation_renew_expire_and_source_capacity() {
	let (mut world, mut movement) = setup(
		r#"{
		"name": "reserve", "seed": 1,
		"rooms": [
			{ "name": "W5N8", "terrain": { "kind": "flat" },
			  "sources": { "positions": [[10, 10]] },
			  "controller": { "level": 0, "pos": [28, 27], "mine": false } },
			{ "name": "W4N8", "terrain": { "kind": "flat" }, "sources": { "positions": [[10, 10]] } }
		]
	}"#,
	);
	let creep = add_creep(&mut world, "R", 27, 27, &["claim", "claim", "move"]);
	let ctrl = world.rooms["W5N8"].objects.values().find(|o| o.obj_type == "controller").unwrap().id;
	let src_cap = |w: &World, room: &str| w.rooms[room].objects.values().find(|o| o.obj_type == "source").unwrap().energy_capacity;
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(src_cap(&world, "W5N8"), 1500, "中立房 1500");
	assert_eq!(src_cap(&world, "W4N8"), 4000, "无控制器 (SK 房) 4000");
	let json = format!(r#"[{{"o":"{creep:024x}","n":"reserveController","d":{{"id":"{ctrl:024x}"}}}}]"#);
	for _ in 0..10 {
		tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	}
	let left = |w: &World| w.rooms["W5N8"].objects[&ctrl].reservation_end_time.map(|e| e as i64 - w.tick as i64);
	// 2 CLAIM × 10 tick 续 = 20, 期间流逝 10 → 剩 ~11 (含首次 endTime = now+1)
	let remaining = left(&world).expect("应处于保留");
	assert!((10..=12).contains(&remaining), "续保留应累积, 剩余 {remaining}");
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(src_cap(&world, "W5N8"), 3000, "被保留 → 3000");
	for _ in 0..15 {
		tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	}
	assert_eq!(left(&world), None, "不续则到期清除");
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	assert_eq!(src_cap(&world, "W5N8"), 1500, "到期回中立 1500");
}

/// 多房间移动仲裁: 本房被挡的移动只取消本房的, 不能误删另一房间同坐标的移动
#[test]
fn blocked_move_cancels_only_same_room() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	// A 在 W4N8 (先仲裁), id 最小, 走向空格 (10,10)
	let a = add_creep(&mut world, "A", 10, 11, &["move"]);
	let obj = world.rooms.get_mut("W5N8").unwrap().objects.remove(&a).unwrap();
	world.rooms.get_mut("W4N8").unwrap().objects.insert(a, obj);
	// B 在 W5N8 走向被静止 creep 占着的 (10,10)
	let b = add_creep(&mut world, "B", 10, 11, &["move"]);
	let _blocker = add_creep(&mut world, "Blocker", 10, 10, &["move"]);
	let json = format!(
		r#"[{{"o":"{a:024x}","n":"move","d":{{"direction":1}}}},{{"o":"{b:024x}","n":"move","d":{{"direction":1}}}}]"#
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	let pos = |room: &str, id: u64| (world.rooms[room].objects[&id].x, world.rooms[room].objects[&id].y);
	assert_eq!(pos("W4N8", a), (10, 10), "W4N8 的 A 不受 W5N8 的阻挡影响");
	assert_eq!(pos("W5N8", b), (10, 11), "W5N8 的 B 被挡住, 不能叠到 Blocker 身上");
}

/// suicide 在意图阶段立即生效 (官方 _die 删除 roomObjects): 同 tick 别的 creep 可以走进它的格子
#[test]
fn suicide_frees_tile_same_tick() {
	let (mut world, mut movement) = setup(TWO_ROOMS);
	let dead = add_creep(&mut world, "Dead", 10, 10, &["move"]);
	let walker = add_creep(&mut world, "Walker", 10, 11, &["move"]);
	let json = format!(
		r#"[{{"o":"{dead:024x}","n":"suicide","d":{{}}}},{{"o":"{walker:024x}","n":"move","d":{{"direction":1}}}}]"#
	);
	tick::run_tick(&mut world, &mut movement, &IntentList::parse(&json));
	assert!(!world.rooms["W5N8"].objects.contains_key(&dead));
	let w = &world.rooms["W5N8"].objects[&walker];
	assert_eq!((w.x, w.y), (10, 10), "自杀者的格子同 tick 空出");
	assert_eq!(world.creeps_lost, 1);
}

// ═════════════════════════ 控制器: activateSafeMode / unclaim (官方 controllers/*.js) ═════════════════════════

fn ctrl_intent(world: &World, name: &str) -> IntentList {
	let id = world.rooms["W5N8"].controller().unwrap().id;
	IntentList::parse(&format!(r#"[{{"o":"{id:024x}","n":"{name}","d":{{}}}}]"#))
}

/// activateSafeMode: 消耗 1 次, safeMode = tick + 20000, 冷却 50000; 次数为 0 / 冷却中 / 升级封锁 → 无效
#[test]
fn activate_safe_mode_and_cooldown() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	let it = ctrl_intent(&world, "activateSafeMode");
	tick::run_tick(&mut world, &mut movement, &it);
	assert_eq!(world.rooms["W5N8"].controller().unwrap().safe_mode, None, "safeModeAvailable=0 → 无效");
	world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap().safe_mode_available = 2;
	let t = world.tick;
	let it = ctrl_intent(&world, "activateSafeMode");
	tick::run_tick(&mut world, &mut movement, &it);
	let c = world.rooms["W5N8"].controller().unwrap();
	assert_eq!(c.safe_mode, Some(t + constants::SAFE_MODE_DURATION));
	assert_eq!(c.safe_mode_cooldown, Some(t + constants::SAFE_MODE_COOLDOWN));
	assert_eq!(c.safe_mode_available, 1);
	assert!(!c.safe_mode_activated, "标记只在本 tick 有效");
	let it = ctrl_intent(&world, "activateSafeMode");
	tick::run_tick(&mut world, &mut movement, &it);
	assert_eq!(world.rooms["W5N8"].controller().unwrap().safe_mode_available, 1, "冷却中 → 无效");
	// 升级封锁中 → 无效
	let (mut world, mut movement) = setup(RCL3_ROOM);
	tick::run_tick(&mut world, &mut movement, &IntentList { entries: vec![] });
	{
		let c = world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap();
		c.safe_mode_available = 1;
		c.upgrade_blocked = 500;
	}
	let it = ctrl_intent(&world, "activateSafeMode");
	tick::run_tick(&mut world, &mut movement, &it);
	assert_eq!(world.rooms["W5N8"].controller().unwrap().safe_mode_available, 1);
}

/// unclaim: 控制器归零并进入安全模式冷却
#[test]
fn unclaim_resets_controller() {
	let (mut world, mut movement) = setup(RCL3_ROOM);
	{
		let c = world.rooms.get_mut("W5N8").unwrap().controller_mut().unwrap();
		c.progress = 777;
		c.safe_mode_available = 3;
		c.safe_mode = Some(9999);
	}
	let t = world.tick;
	let it = ctrl_intent(&world, "unclaim");
	tick::run_tick(&mut world, &mut movement, &it);
	let c = world.rooms["W5N8"].controller().unwrap();
	assert!(!c.mine);
	assert_eq!((c.level, c.progress, c.downgrade_time, c.safe_mode, c.safe_mode_available), (0, 0, None, None, 0));
	assert_eq!(c.safe_mode_cooldown, Some(t + constants::SAFE_MODE_COOLDOWN));
}
