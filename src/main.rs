//! screeps-sim CLI: 场景驱动的高速 Screeps 模拟器。
//!
//! 用法:
//!   screeps-sim --scenario <json|-> [--bundle dist/main.js] [--ticks N]
//!            [--csv out.csv] [--json-report out.json|--> [--quiet]
//!
//! `--scenario -` 从 stdin 读场景; `--json-report -` 把 JSON 报告写到 stdout
//! (此时人类可读报告转 stderr, stdout 保持纯 JSON 供上层工具解析)。

use screeps_sim::engine::cpu::CpuState;
use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::metrics::{evaluate_asserts, MetricsCollector, TickMetrics};
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::scenario::Scenario;
use screeps_sim::engine::snapshot::{build_snapshot, SnapshotOptions};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;
use screeps_sim::runtime::BotRuntime;
use std::io::Write;
use std::time::Instant;

/// CLI 用法: `--help` 走 stdout (供 `screeps-sim --help | less`), 无参数/参数错误走 stderr。
/// 用 stdout 的 `quiet` 表示"正常查询", 只影响第一行的流向。
fn print_usage(quiet: bool) {
    let out: &mut dyn Write = if quiet {
        &mut std::io::stdout()
    } else {
        &mut std::io::stderr()
    };
    let _ = writeln!(
        out,
        "screeps-sim {} — Screeps 本地高速模拟器引擎",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "用法:");
    let _ = writeln!(out, "  screeps-sim --scenario <json|-> [选项]");
    let _ = writeln!(out);
    let _ = writeln!(out, "选项:");
    let _ = writeln!(
        out,
        "  -s, --scenario <json|->     场景文件 (必填); - 从 stdin 读 (npm runner 的管道协议)"
    );
    let _ = writeln!(
        out,
        "  -b, --bundle <js>           bot bundle, 默认 ../dist/main.js"
    );
    let _ = writeln!(out, "  -t, --ticks <n>             覆盖场景里的 tick 数");
    let _ = writeln!(out, "      --csv <path>            逐 tick 指标 CSV");
    let _ = writeln!(out, "      --json-report <path|->  汇总 JSON 报告 (含断言结果); - 写 stdout, 人类可读报告转 stderr");
    let _ = writeln!(
        out,
        "      --record-intents <path> 每 tick 意图流落盘 JSONL (A/B 对比 / 回放原料)"
    );
    let _ = writeln!(
        out,
        "      --replay <path>         纯引擎回放意图流, 零 JS/零 V8, 不需要 --bundle"
    );
    let _ = writeln!(out, "      --dump-memory <path>    落盘最终 Memory");
    let _ = writeln!(out, "      --render-terrain <room> 打印房间 ASCII 地形");
    let _ = writeln!(
        out,
        "      --deterministic-cpu     确定性 CPU 计量 (同场景逐位可复现)"
    );
    let _ = writeln!(out, "      --no-seed-random        关闭 Math.random 播种");
    let _ = writeln!(out, "  -q, --quiet                 只输出报告");
    let _ = writeln!(out, "  -v, --verbose               打印全部 console.log");
    let _ = writeln!(out, "  -V, --version               打印版本");
    let _ = writeln!(out, "  -h, --help                  本帮助");
}

/// 分相耗时累计 (快照序列化 / JS 执行含结果解析 / 引擎结算 + shim 内部细分)
#[derive(Default)]
struct PhaseTiming {
    snapshot_us: u64,
    js_us: u64,
    engine_us: u64,
    js_parse_us: u64,
    js_rebuild_us: u64,
    js_finish_us: u64,
    js_timed_ticks: u64,
}

impl PhaseTiming {
    fn summary(&self, ticks: u64) -> serde_json::Value {
        let per = |us: u64| {
            let us = us as f64 / ticks.max(1) as f64;
            serde_json::json!({ "avgMs": (us / 1000.0 * 1000.0).round() / 1000.0 })
        };
        let jt = self.js_timed_ticks.max(1) as f64;
        serde_json::json!({
            "snapshot": per(self.snapshot_us),
            "js": {
                "avgMs": (self.js_us as f64 / ticks.max(1) as f64 / 1000.0 * 1000.0).round() / 1000.0,
                "parseMs": (self.js_parse_us as f64 / jt / 1000.0 * 1000.0).round() / 1000.0,
                "rebuildMs": (self.js_rebuild_us as f64 / jt / 1000.0 * 1000.0).round() / 1000.0,
                "finishMs": (self.js_finish_us as f64 / jt / 1000.0 * 1000.0).round() / 1000.0,
            },
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
    let mut deterministic_cpu = false;
    // 意图录制/回放 (P1-3): 录制 = 每 tick 意图落盘 JSONL; 回放 = 纯引擎重放, 零 JS
    let mut record_intents_path: Option<String> = None;
    let mut replay_path: Option<String> = None;

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
            "--deterministic-cpu" => {
                deterministic_cpu = true;
                i += 1;
            }
            "--record-intents" => {
                record_intents_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--replay" => {
                replay_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--version" | "-V" => {
                println!("screeps-sim {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                print_usage(true);
                std::process::exit(0);
            }
            _ => {
                eprintln!("未知参数: {} (--help 看全部选项)", args[i]);
                std::process::exit(2);
            }
        }
    }
    if scenario_path.is_empty() {
        print_usage(false);
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
    let scenario = Scenario::from_json(&scenario_text).unwrap_or_else(|e| panic!("{e}"));
    let total_ticks = ticks_override.or(scenario.ticks).unwrap_or(1000);
    if record_intents_path.is_some() && replay_path.is_some() {
        eprintln!("--record-intents 与 --replay 不能同时使用");
        std::process::exit(2);
    }
    // 回放模式: 意图流来自录制文件, 不需要 bot bundle 与 V8
    let replaying = replay_path.is_some();

    let bundle = if replaying {
        String::new()
    } else {
        std::fs::read_to_string(&bundle_path)
            .unwrap_or_else(|e| panic!("读取 bundle 失败 {bundle_path}: {e}"))
    };

    let mut world =
        screeps_sim::engine::scenario::build_world(&scenario).unwrap_or_else(|e| panic!("{e}"));

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
    // 回放模式不需要 V8; 录制文件逐行读取 (tick → intents)
    let mut bot = if replaying {
        None
    } else {
        let mut rt = BotRuntime::new(&bundle, seed).unwrap_or_else(|e| panic!("{e}"));
        // 确定性 CPU: 场景 cpu.deterministic 或 --deterministic-cpu
        let step =
            scenario
                .cpu_virtual_step()
                .or(if deterministic_cpu { Some(0.01) } else { None });
        if let Some(step) = step {
            rt.set_virtual_cpu_step(step);
        }
        Some(rt)
    };
    let mut replay_reader = replay_path.as_ref().map(|p| {
        std::io::BufReader::new(
            std::fs::File::open(p).unwrap_or_else(|e| panic!("打开回放文件失败 {p}: {e}")),
        )
    });
    let mut record_writer = record_intents_path.as_ref().map(|p| {
        std::io::BufWriter::new(
            std::fs::File::create(p).unwrap_or_else(|e| panic!("创建意图录制文件失败 {p}: {e}")),
        )
    });

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

    let mut csv_file = csv_path
        .as_ref()
        .and_then(|p| std::fs::File::create(p).ok());
    if let Some(f) = csv_file.as_mut() {
        let _ = writeln!(
            f,
            "tick,cpu,intents,creeps,energyAvailable,storage,rcl,progress,errors"
        );
    }

    let mut intent_histogram: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    let (mut total_moves, mut total_moved) = (0usize, 0usize);
    let mut run = 0u64;
    let mut fatal_error = false;
    // 分相耗时 (微秒累计): 定位模拟器自身开销 (P0 优化基线)
    let mut phases = PhaseTiming::default();
    while run < total_ticks {
        // 0. 全局重置 (isolate 重建; 全局变量丢失、Memory/内存段保留)
        if let (Some(every), Some(bot)) = (global_reset_every, bot.as_mut()) {
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
        //     (回放模式无 CPU 消耗, 永不触发)
        if !replaying && cpu.should_skip() {
            eprintln!(
                "[t{}] Script execution has been terminated: CPU bucket is empty ({:.0})",
                world.tick, cpu.raw_bucket
            );
            cpu.skip_tick();
            let empty = IntentList::parse("[]");
            if let Some(w) = record_writer.as_mut() {
                let _ = writeln!(
                    w,
                    "{}",
                    serde_json::json!({"tick": world.tick, "intents": []})
                );
            }
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
                let _ = writeln!(
                    f,
                    "{},{:.3},{},{},{},{},{},{},1",
                    world.tick, 0.0, 0, creeps, energy_available, storage_energy, rcl, progress
                );
            }
            run += 1;
            continue;
        }

        // 1. 快照 + bot 执行 (回放模式跳过: 意图流直接来自录制文件)
        let tick_limit = tick_limit_override.unwrap_or_else(|| cpu.tick_limit());
        let (cpu_used, tick_result);
        if replaying {
            cpu_used = 0.0;
            tick_result = screeps_sim::runtime::TickResult {
                intents: read_replay_intents(&mut replay_reader),
                memory: memory_json.clone(),
                cpu_used: 0.0,
                errors: Vec::new(),
                console: Vec::new(),
                timed_out: false,
                js_timing: None,
            };
        } else {
            let bot = bot.as_mut().expect("非回放模式必有 runtime");
            let snapshot_opts = SnapshotOptions {
                cpu: screeps_sim::engine::snapshot::CpuView {
                    limit: cpu_limit,
                    tick_limit,
                    bucket: cpu.displayed_bucket(),
                },
                always_visible: scenario.always_visible,
                // terrain 不可变: 仅 isolate 新建/重置后的首个快照携带 (shim 缓存常驻)
                include_terrain: bot.take_terrain_pending(),
            };
            let t_snap = Instant::now();
            let snapshot = build_snapshot(&world, &snapshot_opts);
            phases.snapshot_us += t_snap.elapsed().as_micros() as u64;

            // bot 执行 (enforce 时按当前 tickLimit 强制中断)。快照/内存按值转移, 避免边界拷贝。
            let tick_timeout = if enforce_cpu { Some(tick_limit) } else { None };
            let t_js = Instant::now();
            let result = match bot.run_tick_limited(
                snapshot,
                std::mem::take(&mut memory_json),
                tick_timeout,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[tick {}] 运行时致命错误: {e}", world.tick);
                    // memory 已按值移交 runtime, 从 IO 通道取回上一 tick 状态 (dump-memory 用)
                    memory_json = bot.pending_memory();
                    // 致命错误计入统计: 断言基于完整事实, 不再"带着错误通过"
                    let (
                        rcl,
                        progress,
                        progress_total,
                        energy_available,
                        storage_energy,
                        creeps,
                        spawning,
                    ) = summarize(&world);
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
            cpu_used = result.cpu_used;
            if let Some(t) = result.js_timing {
                phases.js_parse_us += (t.parse * 1000.0) as u64;
                phases.js_rebuild_us += (t.rebuild * 1000.0) as u64;
                phases.js_finish_us += (t.finish * 1000.0) as u64;
                phases.js_timed_ticks += 1;
            }
            tick_result = result;
        }
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
        if let Some(w) = record_writer.as_mut() {
            let arr: Vec<serde_json::Value> = intents
                .entries
                .iter()
                .map(|(o, n, d)| serde_json::json!({ "o": o, "n": n, "d": d }))
                .collect();
            let _ = writeln!(
                w,
                "{}",
                serde_json::json!({ "tick": world.tick, "intents": arr })
            );
        }
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
                world.tick,
                cpu_used,
                intent_count,
                creeps,
                energy_available,
                storage_energy,
                rcl,
                progress,
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
    // 录制文件落盘 (BufWriter 显式 flush, 不可忽略错误)
    if let Some(w) = record_writer.as_mut() {
        let _ = std::io::Write::flush(w);
    }
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
        "分相耗时/tick: 快照 {:.3}ms · JS {:.3}ms (parse {:.3} / 重建 {:.3} / 收尾 {:.3}) · 引擎 {:.3}ms (状态哈希 0x{:016x})",
        phases.snapshot_us as f64 / 1000.0 / actual_ticks.max(1) as f64,
        phases.js_us as f64 / 1000.0 / actual_ticks.max(1) as f64,
        phases.js_parse_us as f64 / 1000.0 / phases.js_timed_ticks.max(1) as f64,
        phases.js_rebuild_us as f64 / 1000.0 / phases.js_timed_ticks.max(1) as f64,
        phases.js_finish_us as f64 / 1000.0 / phases.js_timed_ticks.max(1) as f64,
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

/// 回放文件的行格式: {"tick":T,"intents":[{o,n,d},...]}
#[derive(serde::Deserialize)]
struct ReplayLine {
    #[allow(dead_code)]
    tick: u64,
    #[serde(default)]
    intents: Vec<screeps_sim::engine::intents::IntentEntry>,
}

/// 读取回放文件的下一行意图 (EOF / 空行 / 坏行 → 空意图, 世界照常推进)
fn read_replay_intents(
    reader: &mut Option<std::io::BufReader<std::fs::File>>,
) -> screeps_sim::engine::intents::IntentList {
    use std::io::BufRead as _;
    let Some(r) = reader else {
        return screeps_sim::engine::intents::IntentList {
            entries: Vec::new(),
        };
    };
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) => {
                return screeps_sim::engine::intents::IntentList {
                    entries: Vec::new(),
                }
            }
            Ok(_) => {}
            Err(_) => {
                return screeps_sim::engine::intents::IntentList {
                    entries: Vec::new(),
                }
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ReplayLine>(&line) {
            Ok(l) => return screeps_sim::engine::intents::IntentList::from_entries(l.intents),
            Err(_) => continue,
        }
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
                "storage" if o.mine => {
                    storage_energy += o.store.get("energy").copied().unwrap_or(0)
                }
                _ => {}
            }
        }
    }
    (
        rcl,
        progress,
        progress_total,
        energy_available,
        storage_energy,
        creeps,
        spawning,
    )
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
