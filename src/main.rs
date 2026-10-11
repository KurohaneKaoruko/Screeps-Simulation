//! nexus-sim CLI: 场景驱动的高速 Screeps 模拟器。
//!
//! 用法:
//!   nexus-sim --scenario <json> [--bundle dist/main.js] [--ticks N]
//!            [--csv out.csv] [--json-report out.json] [--quiet]

use nexus_sim::runtime::BotRuntime;
use nexus_sim::engine::intents::IntentList;
use nexus_sim::engine::metrics::{evaluate_asserts, MetricsCollector, TickMetrics};
use nexus_sim::engine::movement::Movement;
use nexus_sim::engine::scenario::Scenario;
use nexus_sim::engine::snapshot::{build_snapshot, SnapshotOptions};
use nexus_sim::engine::tick;
use nexus_sim::engine::world::World;
use std::io::Write as _;
use std::time::Instant;

fn main() {
    // deno_core 的 V8 isolate 需要 tokio 运行时上下文 (延迟任务调度)
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime 创建失败");
    tokio_rt.block_on(async_main());
}

async fn async_main() {
    let args: Vec<String> = std::env::args().collect();
    let mut scenario_path = String::new();
    let mut bundle_path = "../dist/main.js".to_string();
    let mut ticks_override: Option<u64> = None;
    let mut csv_path: Option<String> = None;
    let mut report_path: Option<String> = None;
    let mut quiet = false;
    let mut verbose = false;
    let mut dump_memory_path: Option<String> = None;
    let mut render_room: Option<String> = None;
    let mut seed_random = true;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" | "-s" => {
                scenario_path = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--bundle" | "-b" => {
                bundle_path = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--ticks" | "-t" => {
                ticks_override = args.get(i + 1).and_then(|v| v.parse().ok());
                i += 2;
            }
            "--csv" => {
                csv_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--json-report" => {
                report_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--quiet" | "-q" => {
                quiet = true;
                i += 1;
            }
            "--verbose" | "-v" => {
                verbose = true;
                i += 1;
            }
            "--render-terrain" => {
                render_room = args.get(i + 1).cloned();
                i += 2;
            }
            "--dump-memory" => {
                dump_memory_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--no-seed-random" => {
                seed_random = false;
                i += 1;
            }
            _ => {
                eprintln!("未知参数: {} (支持 --scenario --bundle --ticks --csv --json-report --quiet)", args[i]);
                std::process::exit(2);
            }
        }
    }
    if scenario_path.is_empty() {
        eprintln!("用法: nexus-sim --scenario <scenario.json> [--bundle dist/main.js] [--ticks N] [--csv out.csv] [--json-report out.json] [--quiet]");
        std::process::exit(2);
    }

    let scenario_text = std::fs::read_to_string(&scenario_path)
        .unwrap_or_else(|e| panic!("读取场景失败 {scenario_path}: {e}"));
    let scenario = Scenario::from_json(&scenario_text)
        .unwrap_or_else(|e| panic!("{e}"));
    let total_ticks = ticks_override.or(scenario.ticks).unwrap_or(1000);

    let bundle = std::fs::read_to_string(&bundle_path)
        .unwrap_or_else(|e| panic!("读取 bundle 失败 {bundle_path}: {e}"));

    let mut world = nexus_sim::engine::scenario::build_world(&scenario)
        .unwrap_or_else(|e| panic!("{e}"));

    // 地形可视化 (场景作者核对生成质量)
    if let Some(room_name) = &render_room {
        let Some(room) = world.rooms.get(room_name) else {
            eprintln!("场景中没有房间 {room_name}");
            std::process::exit(2);
        };
        let mut walls = 0usize;
        let mut swamps = 0usize;
        for y in 0..50 {
            for x in 0..50 {
                if room.terrain.is_wall(x, y) {
                    walls += 1;
                }
                if room.terrain.is_swamp(x, y) {
                    swamps += 1;
                }
            }
        }
        println!("地形 {room_name}: 墙 {}/2500 ({:.1}%), 沼泽 {}/2500 ({:.1}%)", walls, 100.0 * walls as f32 / 2500.0, swamps, 100.0 * swamps as f32 / 2500.0);
        println!("{}", room.terrain.render());
    }
    let mut movement = Movement::default();
    let mut metrics = MetricsCollector::default();

    let seed = if seed_random || scenario.deterministic_random {
        scenario.seed.unwrap_or(1)
    } else {
        0
    };
    let mut bot = BotRuntime::new(&bundle, seed).unwrap_or_else(|e| panic!("{e}"));

    let wall_start = Instant::now();
    let mut memory_json = scenario.memory.clone().unwrap_or_else(|| "{}".to_string());
    let mut bucket = scenario.cpu_bucket();
    let cpu_limit = scenario.cpu_limit();
    let tick_limit = scenario.cpu_tick_limit();

    if !quiet {
        println!(
            "▸ 场景 {} | 房间 {} | tick {} | bundle {}",
            scenario.name,
            scenario.rooms.len(),
            total_ticks,
            bundle_path
        );
    }

    let mut csv_file = csv_path.as_ref().and_then(|p| std::fs::File::create(p).ok());
    if let Some(f) = csv_file.as_mut() {
        let _ = writeln!(f, "tick,cpu,intents,creeps,energyAvailable,storage,rcl,progress,errors");
    }

    let mut intent_histogram: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let (mut total_moves, mut total_moved) = (0usize, 0usize);
    let mut run = 0u64;
    while run < total_ticks {
        // 1. 快照
        let snapshot_opts = SnapshotOptions {
            cpu: nexus_sim::engine::snapshot::CpuView {
                limit: cpu_limit,
                tick_limit: bucket.max(cpu_limit).min(500.0).max(cpu_limit),
                bucket,
            },
            always_visible: scenario.always_visible,
        };
        let snapshot = build_snapshot(&world, &snapshot_opts);

        // 2. bot 执行
        let tick_result = match bot.run_tick(&snapshot.to_string(), &memory_json) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[tick {}] 运行时致命错误: {e}", world.tick);
                break;
            }
        };
        let cpu_used = tick_result.cpu_used;

        // 3. 控制台与错误输出
        if verbose {
            for line in &tick_result.console {
                println!("[t{}] {}", world.tick, line);
            }
        } else if !quiet {
            for line in &tick_result.console {
                if line.starts_with("[nexus][error") || line.contains("✗") {
                    println!("[t{}] {}", world.tick, line);
                }
            }
        }
        for err in &tick_result.errors {
            if !quiet || metrics.total_errors < 10 {
                eprintln!("[t{}] bot 异常:\n{}", world.tick, err);
            }
        }

        // 4. 意图结算 + tick 推进
        let intents = IntentList::parse(&tick_result.intents_json);
        let intent_count = intents.entries.len();
        for (_, name, _) in &intents.entries {
            *intent_histogram.entry(name.clone()).or_insert(0) += 1;
        }
        let outcome = tick::run_tick(&mut world, &mut movement, &intents);
        total_moves += outcome.move_stats.0;
        total_moved += outcome.move_stats.1;

        // 5. bucket 结算
        bucket = (bucket + (cpu_limit - cpu_used)).clamp(0.0, 10_000.0);

        // 6. 指标
        let (rcl, progress, progress_total, energy_available, storage_energy, creeps, spawning) =
            summarize(&world);
        metrics.record(
            TickMetrics {
                tick: world.tick,
                cpu_used,
                intents: intent_count,
                creeps,
                spawning,
                energy_available,
                storage_energy,
                rcl,
                progress,
                controller_progress_total: progress_total,
                errors: tick_result.errors.len(),
                console_lines: tick_result.console.len(),
            },
            tick_result.memory.len(),
            cpu_used > tick_limit,
        );
        memory_json = tick_result.memory;

        if let Some(f) = csv_file.as_mut() {
            let _ = writeln!(
                f,
                "{},{:.3},{},{},{},{},{},{},{}",
                world.tick, cpu_used, intent_count, creeps, energy_available, storage_energy, rcl, progress,
                tick_result.errors.len()
            );
        }

        if !quiet && (world.tick) % (total_ticks / 20).max(1) == 0 {
            println!(
                "  t{:<7} RCL{} 进度{:<8} 单位{:<3} 可用能量{:<6} cpu均值{:.2}ms",
                world.tick,
                rcl,
                progress,
                creeps,
                energy_available,
                metrics.cpu_avg()
            );
            let _ = &outcome.creep_deaths;
        }
        run += 1;
    }

    let elapsed = wall_start.elapsed();
    let tps = total_ticks as f64 / elapsed.as_secs_f64().max(1e-9);

    // ── 报告 ──
    println!("\n══════════ 模拟报告 ══════════");
    println!("场景: {}  tick: {}  用时: {:.2}s  ({:.0} tick/s)", scenario.name, total_ticks, elapsed.as_secs_f64(), tps);
    println!("CPU: 均值 {:.3}ms  P50 {:.3}ms  P95 {:.3}ms  P99 {:.3}ms  超限 tick {}",
        metrics.cpu_avg(), metrics.cpu_percentile(0.50), metrics.cpu_percentile(0.95),
        metrics.cpu_percentile(0.99), metrics.cpu_over_limit_ticks);
    println!("能量: 采集 {} 建造 {} 升级 {} 孵化 {}",
        world.energy_harvested, world.energy_built, world.energy_control, world.energy_spawned);
    println!("单位: 生产 {} 死亡 {} 存活 {}",
        world.creeps_produced,
        world.creeps_lost,
        metrics.history.last().map(|m| m.creeps).unwrap_or(0));
    println!("GCL: {} 级 ({}/{})",
        world.gcl_level(),
        world.gcl_level_progress().0,
        world.gcl_level_progress().1);
    print!("RCL 时间线: ");
    for (level, t) in &world.rcl_timeline {
        print!("L{level}@{t} ");
    }
    println!();
    println!("Memory 峰值: {} 字节", metrics.memory_peak);
    println!("意图分布: {:?}", intent_histogram);
    println!("移动执行率: {}/{} = {:.1}%", total_moved, total_moves, 100.0 * total_moved as f64 / total_moves.max(1) as f64);

    // ── 落盘输出 (先于断言, 保证失败时也能拿到数据) ──
    if let Some(path) = &dump_memory_path {
        let _ = std::fs::write(path, &memory_json);
    }
    if let Some(path) = &report_path {
        let report = build_json_report(&world, &metrics, &scenario, tps, elapsed.as_secs_f64());
        if let Ok(text) = serde_json::to_string_pretty(&report) {
            let _ = std::fs::write(path, text);
        }
    }

    // ── 断言 ──
    if !scenario.asserts.is_empty() {
        println!("\n── 场景断言 ──");
        let results = evaluate_asserts(&world, &metrics, &scenario.asserts);
        let mut all_pass = true;
        for r in &results {
            let tag = if r.passed { "✓ PASS" } else { "✗ FAIL" };
            if !r.passed {
                all_pass = false;
            }
            let label = match &r.spec {
                nexus_sim::engine::scenario::AssertSpec::RclAtLeast { .. } => "rclAtLeast",
                nexus_sim::engine::scenario::AssertSpec::RclReachedBy { .. } => "rclReachedBy",
                nexus_sim::engine::scenario::AssertSpec::CreepCountAtLeast { .. } => "creepCountAtLeast",
                nexus_sim::engine::scenario::AssertSpec::CpuAvgBelow { .. } => "cpuAvgBelow",
                nexus_sim::engine::scenario::AssertSpec::CpuP95Below { .. } => "cpuP95Below",
                nexus_sim::engine::scenario::AssertSpec::NoErrors => "noErrors",
            };
            println!("{tag}  {label}  ({})", r.detail);
        }
        if !all_pass {
            std::process::exit(1);
        }
    }

}

fn summarize(world: &World) -> (u8, u32, u32, u32, u32, usize, usize) {
    let mut rcl = 0;
    let mut progress = 0;
    let mut progress_total = 0;
    let mut energy_available = 0;
    let mut storage_energy = 0;
    let mut creeps = 0;
    let mut spawning = 0;
    for room in world.rooms.values() {
        if let Some(c) = room.controller() {
            if c.mine {
                rcl = rcl.max(c.level);
                progress = c.progress;
                progress_total = nexus_sim::engine::constants::CONTROLLER_LEVELS
                    .iter()
                    .find(|(l, _)| *l == c.level)
                    .map(|(_, p)| *p)
                    .unwrap_or(0);
            }
        }
        energy_available += room.energy_available();
        for o in room.objects.values() {
            match o.obj_type.as_str() {
                "creep" if !o.spawning => creeps += 1,
                "creep" => spawning += 1,
                "storage" if o.mine => storage_energy += o.store.get("energy").copied().unwrap_or(0),
                _ => {}
            }
        }
    }
    (rcl, progress, progress_total, energy_available, storage_energy, creeps, spawning)
}

fn build_json_report(
    world: &World,
    metrics: &MetricsCollector,
    scenario: &Scenario,
    tps: f64,
    seconds: f64,
) -> serde_json::Value {
    use serde_json::json;
    let rcl_timeline: serde_json::Value = world
        .rcl_timeline
        .iter()
        .map(|(l, t)| (l.to_string(), json!(t)))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();
    json!({
        "scenario": scenario.name,
        "ticks": metrics.history.len(),
        "seconds": seconds,
        "ticksPerSecond": tps,
        "cpu": {
            "avg": metrics.cpu_avg(),
            "p50": metrics.cpu_percentile(0.50),
            "p95": metrics.cpu_percentile(0.95),
            "p99": metrics.cpu_percentile(0.99),
            "overLimitTicks": metrics.cpu_over_limit_ticks,
        },
        "energy": {
            "harvested": world.energy_harvested,
            "built": world.energy_built,
            "control": world.energy_control,
            "spawned": world.energy_spawned,
        },
        "creeps": {
            "produced": world.creeps_produced,
            "lost": world.creeps_lost,
            "alive": metrics.history.last().map(|m| m.creeps).unwrap_or(0),
        },
        "gcl": {
            "level": world.gcl_level(),
            "progress": world.gcl_level_progress().0,
            "progressTotal": world.gcl_level_progress().1,
        },
        "rclTimeline": rcl_timeline,
        "memoryPeakBytes": metrics.memory_peak,
        "totalErrors": metrics.total_errors,
        "final": metrics.history.last().map(|m| json!({
            "tick": m.tick,
            "rcl": m.rcl,
            "progress": m.progress,
            "creeps": m.creeps,
            "energyAvailable": m.energy_available,
            "storage": m.storage_energy,
        })).unwrap_or(json!(null)),
    })
}
