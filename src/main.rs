//! screeps-sim CLI: 场景驱动的高速 Screeps 模拟器。
//!
//! 用法:
//!   screeps-sim --scenario <json|-> [--bundle dist/main.js] [--ticks N]
//!            [--csv out.csv] [--json-report out.json|--> [--quiet]
//!
//! `--scenario -` 从 stdin 读场景; `--json-report -` 把 JSON 报告写到 stdout
//! (此时人类可读报告转 stderr, stdout 保持纯 JSON 供上层工具解析)。

use screeps_sim::runtime::BotRuntime;
use screeps_sim::engine::cpu::CpuState;
use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::metrics::{evaluate_asserts, MetricsCollector, TickMetrics};
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::scenario::Scenario;
use screeps_sim::engine::snapshot::{build_snapshot, SnapshotOptions};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;
use std::io::Write as _;
use std::time::Instant;

/// 分相耗时累计 (快照序列化 / JS 执行含结果解析 / 引擎结算)
#[derive(Default)]
struct PhaseTiming {
    snapshot_us: u64,
    js_us: u64,
    engine_us: u64,
}

impl PhaseTiming {
    fn summary(&self, ticks: u64) -> serde_json::Value {
        let per = |us: u64| {
            let us = us as f64 / ticks.max(1) as f64;
            serde_json::json!({ "avgMs": (us / 1000.0 * 1000.0).round() / 1000.0 })
        };
        serde_json::json!({
            "snapshot": per(self.snapshot_us),
            "js": per(self.js_us),
            "engine": per(self.engine_us),
        })
    }
}

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
        eprintln!("用法: screeps-sim --scenario <scenario.json|-> [--bundle dist/main.js] [--ticks N] [--csv out.csv] [--json-report out.json|-] [--quiet]");
        eprintln!("      --scenario -      场景 JSON 从 stdin 读入 (npm runner 的管道协议)");
        eprintln!("      --json-report -   JSON 报告写 stdout, 人类可读报告转 stderr");
        std::process::exit(2);
    }

    // JSON 报告走 stdout 时, 人类可读输出全部转 stderr, 保持 stdout 纯 JSON
    let json_stdout = report_path.as_deref() == Some("-");
    if json_stdout {
        quiet = true;
    }
    let say = |line: String| {
        if json_stdout {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };

    let scenario_text = if scenario_path == "-" {
        use std::io::Read;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .unwrap_or_else(|e| panic!("读取 stdin 场景失败: {e}"));
        text
    } else {
        std::fs::read_to_string(&scenario_path)
            .unwrap_or_else(|e| panic!("读取场景失败 {scenario_path}: {e}"))
    };
    let scenario = Scenario::from_json(&scenario_text)
        .unwrap_or_else(|e| panic!("{e}"));
    let total_ticks = ticks_override.or(scenario.ticks).unwrap_or(1000);

    let bundle = std::fs::read_to_string(&bundle_path)
        .unwrap_or_else(|e| panic!("读取 bundle 失败 {bundle_path}: {e}"));

    let mut world = screeps_sim::engine::scenario::build_world(&scenario)
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
        say(format!(
            "地形 {room_name}: 墙 {}/2500 ({:.1}%), 沼泽 {}/2500 ({:.1}%)",
            walls,
            100.0 * walls as f32 / 2500.0,
            swamps,
            100.0 * swamps as f32 / 2500.0
        ));
        say(room.terrain.render());
    }
    let mut movement = Movement::default();
    let mut metrics = MetricsCollector::default();

    // seed == 0 → 不播种。--no-seed-random 现在真的会关闭播种 (此前被 deterministic_random 短路)。
    let seed = if seed_random && scenario.deterministic_random {
        scenario.seed.unwrap_or(1)
    } else {
        0
    };
    let mut bot = BotRuntime::new(&bundle, seed).unwrap_or_else(|e| panic!("{e}"));

    let wall_start = Instant::now();
    let mut memory_json = scenario.memory.clone().unwrap_or_else(|| "{}".to_string());
    let cpu_limit = scenario.cpu_limit();
    // 官方 bucket 语义 (driver make.js/data.js): 可为负、负数时跳过执行、显示值 max(0, raw)
    let mut cpu = CpuState::new(cpu_limit, scenario.cpu_bucket());
    // 显式覆盖 tickLimit 时不随 bucket 逐 tick 变化
    let tick_limit_override = scenario.cpu_tick_limit_override();
    // CPU 强制中断: 官方 tickLimit 到期即终止脚本 ("Script execution timed out ungracefully")
    let enforce_cpu = scenario.cpu_enforce();
    // 全局重置周期 (官方由 isolate 堆占用触发)
    let global_reset_every = scenario.global_reset_every();

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
    let mut fatal_error = false;
    // 分相耗时 (微秒累计): 定位模拟器自身开销 (P0 优化基线)
    let mut phases = PhaseTiming::default();
    while run < total_ticks {
        // 0. 全局重置 (isolate 重建; 全局变量丢失、Memory/内存段保留)
        if let Some(every) = global_reset_every {
            if world.tick > 0 && world.tick % every == 0 {
                if let Err(e) = bot.reset_global() {
                    eprintln!("[tick {}] 全局重置失败: {e}", world.tick);
                    fatal_error = true;
                    break;
                }
                if !quiet {
                    println!("  t{:<7} 全局重置 (isolate 重建, Memory 保留)", world.tick);
                }
            }
        }
        // 0.5. bucket 耗尽 → 跳过本 tick 的用户执行 (官方 make.js:72-83)。
        //     游戏世界照常推进 (只有 bot 的意图缺席), 跳过期间每 tick 回能。
        if cpu.should_skip() {
            eprintln!(
                "[t{}] Script execution has been terminated: CPU bucket is empty ({:.0})",
                world.tick, cpu.raw_bucket
            );
            cpu.skip_tick();
            let empty = IntentList::parse("[]");
            tick::run_tick(&mut world, &mut movement, &empty);
            let (rcl, progress, progress_total, energy_available, storage_energy, creeps, spawning) =
                summarize(&world);
            metrics.record(
                TickMetrics {
                    tick: world.tick,
                    cpu_used: 0.0,
                    intents: 0,
                    creeps,
                    spawning,
                    energy_available,
                    storage_energy,
                    rcl,
                    progress,
                    controller_progress_total: progress_total,
                    errors: 1,
                    console_lines: 0,
                },
                memory_json.len(),
                false,
            );
            if let Some(f) = csv_file.as_mut() {
                let _ = writeln!(f, "{},{:.3},{},{},{},{},{},{},1", world.tick, 0.0, 0, creeps, energy_available, storage_energy, rcl, progress);
            }
            run += 1;
            continue;
        }

        // 1. 快照 (Game.cpu.bucket 显示 max(0, raw); tickLimit = min(500, bucket + limit))
        let tick_limit = tick_limit_override.unwrap_or_else(|| cpu.tick_limit());
        let snapshot_opts = SnapshotOptions {
            cpu: screeps_sim::engine::snapshot::CpuView {
                limit: cpu_limit,
                tick_limit,
                bucket: cpu.displayed_bucket(),
            },
            always_visible: scenario.always_visible,
        };
        let t_snap = Instant::now();
        let snapshot = build_snapshot(&world, &snapshot_opts).to_string();
        phases.snapshot_us += t_snap.elapsed().as_micros() as u64;

        // 2. bot 执行 (enforce 时按当前 tickLimit 强制中断)。快照/内存按值转移, 避免边界拷贝。
        let tick_timeout = if enforce_cpu { Some(tick_limit) } else { None };
        let t_js = Instant::now();
        let tick_result = match bot.run_tick_limited(snapshot, std::mem::take(&mut memory_json), tick_timeout) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[tick {}] 运行时致命错误: {e}", world.tick);
                // memory 已按值移交 runtime, 从 IO 通道取回上一 tick 状态 (dump-memory 用)
                memory_json = bot.pending_memory();
                // 致命错误计入统计: 断言基于完整事实, 不再"带着错误通过"
                let (rcl, progress, progress_total, energy_available, storage_energy, creeps, spawning) =
                    summarize(&world);
                metrics.record(
                    TickMetrics {
                        tick: world.tick,
                        cpu_used: 0.0,
                        intents: 0,
                        creeps,
                        spawning,
                        energy_available,
                        storage_energy,
                        rcl,
                        progress,
                        controller_progress_total: progress_total,
                        errors: 1,
                        console_lines: 0,
                    },
                    memory_json.len(),
                    false,
                );
                fatal_error = true;
                break;
            }
        };
        phases.js_us += t_js.elapsed().as_micros() as u64;
        let cpu_used = tick_result.cpu_used;
        if tick_result.timed_out {
            eprintln!(
                "[t{}] Script execution timed out ungracefully (tickLimit {:.0}ms): 本 tick 意图与 Memory 改动均丢弃",
                world.tick, tick_limit
            );
        }

        // 3. 控制台与错误输出
        if verbose {
            for line in &tick_result.console {
                say(format!("[t{}] {}", world.tick, line));
            }
        } else if !quiet {
            for line in &tick_result.console {
                if line.starts_with("[nexus][error") || line.contains("✗") {
                    println!("[t{}] {}", world.tick, line);
                }
            }
        }
        for err in &tick_result.errors {
            if tick_result.timed_out {
                continue; // 超时已在上方单独报告
            }
            if !quiet || metrics.total_errors < 10 {
                eprintln!("[t{}] bot 异常:\n{}", world.tick, err);
            }
        }

        // 4. 意图结算 + tick 推进 (意图已在 runtime 单次解析)
        let intents = tick_result.intents;
        let intent_count = intents.entries.len();
        for (_, name, _) in &intents.entries {
            *intent_histogram.entry(name.clone()).or_insert(0) += 1;
        }
        let t_engine = Instant::now();
        let outcome = tick::run_tick(&mut world, &mut movement, &intents);
        phases.engine_us += t_engine.elapsed().as_micros() as u64;
        total_moves += outcome.move_stats.0;
        total_moved += outcome.move_stats.1;

        // 5. bucket 结算 (官方 make.js:193-198: raw = min(10000, raw + limit - cpuUsed), 可为负)
        cpu.settle(cpu_used);

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
            tick_result.timed_out || cpu_used > tick_limit,
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
    // tps 按实际运行的 tick 数计 (致命错误/提前退出时不再假装跑满)
    let actual_ticks = run;
    let tps = actual_ticks as f64 / elapsed.as_secs_f64().max(1e-9);

    // ── 断言先结算 (结果并入 JSON 报告, 供上层 runner 消费) ──
    let assert_results = evaluate_asserts(&world, &metrics, &scenario.asserts);

    // ── 落盘输出 (先于人类报告; 失败时也能拿到数据) ──
    if let Some(path) = &dump_memory_path {
        let _ = std::fs::write(path, &memory_json);
    }
    if let Some(path) = &report_path {
        let report = build_json_report(
            &world,
            &metrics,
            &scenario,
            tps,
            elapsed.as_secs_f64(),
            &assert_results,
            fatal_error,
            phases.summary(actual_ticks),
        );
        if let Ok(text) = serde_json::to_string_pretty(&report) {
            if path == "-" {
                println!("{text}");
            } else {
                let _ = std::fs::write(path, text);
            }
        }
    }

    // ── 人类可读报告 (json_stdout 模式下转 stderr) ──
    say("\n══════════ 模拟报告 ══════════".into());
    say(format!(
        "场景: {}  tick: {}/{}  用时: {:.2}s  ({:.0} tick/s)",
        scenario.name,
        actual_ticks,
        total_ticks,
        elapsed.as_secs_f64(),
        tps
    ));
    say(format!(
        "CPU: 均值 {:.3}ms  P50 {:.3}ms  P95 {:.3}ms  P99 {:.3}ms  超限 tick {}",
        metrics.cpu_avg(),
        metrics.cpu_percentile(0.50),
        metrics.cpu_percentile(0.95),
        metrics.cpu_percentile(0.99),
        metrics.cpu_over_limit_ticks
    ));
    say(format!(
        "能量: 采集 {} 建造 {} 升级 {} 孵化 {}",
        world.energy_harvested, world.energy_built, world.energy_control, world.energy_spawned
    ));
    say(format!(
        "单位: 生产 {} 死亡 {} 存活 {}",
        world.creeps_produced,
        world.creeps_lost,
        metrics.history.last().map(|m| m.creeps).unwrap_or(0)
    ));
    say(format!(
        "GCL: {} 级 ({}/{})",
        world.gcl_level(),
        world.gcl_level_progress().0,
        world.gcl_level_progress().1
    ));
    let mut timeline = String::from("RCL 时间线: ");
    for (level, t) in &world.rcl_timeline {
        timeline.push_str(&format!("L{level}@{t} "));
    }
    say(timeline);
    say(format!("Memory 峰值: {} 字节", metrics.memory_peak));
    say(format!("意图分布: {:?}", intent_histogram));
    say(format!(
        "移动执行率: {}/{} = {:.1}%",
        total_moved,
        total_moves,
        100.0 * total_moved as f64 / total_moves.max(1) as f64
    ));
    say(format!(
        "分相耗时/tick: 快照 {:.3}ms · JS {:.3}ms · 引擎 {:.3}ms (状态哈希 0x{:016x})",
        phases.snapshot_us as f64 / 1000.0 / actual_ticks.max(1) as f64,
        phases.js_us as f64 / 1000.0 / actual_ticks.max(1) as f64,
        phases.engine_us as f64 / 1000.0 / actual_ticks.max(1) as f64,
        world.state_hash()
    ));

    // ── 断言报告 ──
    if !assert_results.is_empty() {
        say("\n── 场景断言 ──".into());
        let mut all_pass = true;
        for r in &assert_results {
            let tag = if r.passed { "✓ PASS" } else { "✗ FAIL" };
            if !r.passed {
                all_pass = false;
            }
            say(format!("{tag}  {}  ({})", assert_kind(&r.spec), r.detail));
        }
        if !all_pass {
            std::process::exit(1);
        }
    }

    // 无断言 (或断言全过) 时, 致命错误仍必须以非零码退出 —— 模拟没有完整跑完
    if fatal_error {
        eprintln!("✗ 模拟因致命运行时错误提前终止 (退出码 1)");
        std::process::exit(1);
    }
}

/// 断言种类标签 (与场景 JSON 的 `kind` 一致)
fn assert_kind(spec: &screeps_sim::engine::scenario::AssertSpec) -> &'static str {
    use screeps_sim::engine::scenario::AssertSpec;
    match spec {
        AssertSpec::RclAtLeast { .. } => "rclAtLeast",
        AssertSpec::RclReachedBy { .. } => "rclReachedBy",
        AssertSpec::CreepCountAtLeast { .. } => "creepCountAtLeast",
        AssertSpec::CpuAvgBelow { .. } => "cpuAvgBelow",
        AssertSpec::CpuP95Below { .. } => "cpuP95Below",
        AssertSpec::NoErrors => "noErrors",
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
                progress_total = screeps_sim::engine::constants::CONTROLLER_LEVELS
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
    asserts: &[screeps_sim::engine::metrics::AssertResult],
    fatal: bool,
    profile: serde_json::Value,
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
        "stateHash": format!("0x{:016x}", world.state_hash()),
        "profile": profile,
        "asserts": asserts
            .iter()
            .map(|r| json!({
                "kind": assert_kind(&r.spec),
                "passed": r.passed,
                "detail": r.detail,
            }))
            .collect::<Vec<_>>(),
        "fatalError": fatal,
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
