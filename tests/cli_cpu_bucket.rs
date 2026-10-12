//! CLI 级集成测试: bucket 耗尽跳过、noErrors 断言、退出码语义。
//! 通过 CARGO_BIN_EXE 直接驱动二进制, 验证 main.rs 主循环的官方 bucket 行为。

use std::io::Write;
use std::process::Command;

fn tmp_path(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("screeps-sim-test-{}-{name}", std::process::id()));
    p
}

fn write_file(path: &std::path::Path, content: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

/// 最小场景: 单房 + spawn; cpu 配置可注入
fn scenario_json(cpu: &str, ticks: u64, asserts: &str) -> String {
    format!(
        r#"{{
            "name": "cli-test",
            "ticks": {ticks},
            "cpu": {cpu},
            "rooms": [{{
                "name": "W5N8",
                "terrain": {{ "kind": "flat" }},
                "controller": {{ "level": 1, "pos": [28, 27] }},
                "structures": [{{ "type": "spawn", "pos": [25, 25], "energy": 300 }}]
            }}],
            "asserts": [{asserts}]
        }}"#
    )
}

/// 正常运行 (小 CPU 消耗) + 无断言 → 退出码 0
#[test]
fn clean_run_exits_zero() {
    let bin = env!("CARGO_BIN_EXE_screeps-sim");
    let scenario_path = tmp_path("clean-scenario.json");
    let bundle_path = tmp_path("clean-bundle.js");
    write_file(
        &scenario_path,
        &scenario_json(r#"{"limit": 100, "bucket": 10000}"#, 3, r#""#),
    );
    write_file(&bundle_path, "exports.loop = function() {};");
    let out = Command::new(bin)
        .arg("--scenario")
        .arg(&scenario_path)
        .arg("--bundle")
        .arg(&bundle_path)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "正常场景应退出 0\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// bucket 耗尽: limit=1ms + 重 CPU bundle → 第 1 tick 后 bucket 为负,
/// 后续 tick 跳过用户执行 (官方 "CPU bucket is empty"), noErrors 断言失败 → 退出码 1。
#[test]
fn bucket_drain_skips_ticks_and_fails_no_errors() {
    let bin = env!("CARGO_BIN_EXE_screeps-sim");
    let scenario_path = tmp_path("drain-scenario.json");
    let bundle_path = tmp_path("drain-bundle.js");
    // 300 万次 Math.random 烧掉远超 1ms 的墙钟时间 (仅计量, 不 enforce)
    write_file(
        &bundle_path,
        r#"exports.loop = function() {
            let s = 0;
            for (let i = 0; i < 3000000; i++) s += Math.random();
            if (s < 0) console.log(s);
        };"#,
    );
    write_file(
        &scenario_path,
        &scenario_json(r#"{"limit": 1, "bucket": 0}"#, 5, r#"{"kind": "noErrors"}"#),
    );
    let out = Command::new(bin)
        .arg("--scenario")
        .arg(&scenario_path)
        .arg("--bundle")
        .arg(&bundle_path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "bucket 耗尽 → noErrors 失败 → 退出码 1\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("CPU bucket is empty"),
        "应出现官方跳过提示, 实际 stderr:\n{stderr}"
    );
    // 5 个 tick: 第 1 个正常执行, 其余被跳过 → 汇报 5/5 且 noErrors FAIL
    assert!(stdout.contains("noErrors"), "断言结果应出现在报告中:\n{stdout}");
}

/// stdin 场景 + `--json-report -`: stdout 必须是纯 JSON (机器可读管道协议),
/// 断言结果内嵌在报告中, 人类可读报告转 stderr, 失败断言 → 退出码 1。
#[test]
fn stdin_scenario_and_stdout_json_report() {
    let bin = env!("CARGO_BIN_EXE_screeps-sim");
    let bundle_path = tmp_path("stdin-bundle.js");
    write_file(&bundle_path, "exports.loop = function() {};");
    // creepCountAtLeast 1 必然失败 (场景无初始单位) —— 覆盖失败路径的管道行为
    let scenario = scenario_json(
        r#"{"limit": 100, "bucket": 10000}"#,
        3,
        r#"{"kind": "noErrors"}, {"kind": "creepCountAtLeast", "value": 1}"#,
    );
    let mut child = Command::new(bin)
        .arg("--scenario")
        .arg("-")
        .arg("--bundle")
        .arg(&bundle_path)
        .arg("--json-report")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(scenario.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(1), "失败断言应退出 1\nstderr: {stderr}");
    // stdout 必须可整体解析为 JSON (人类报告不得混入)
    let report: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout 应为纯 JSON ({e}):\n{stdout}"));
    assert_eq!(report["scenario"], "cli-test");
    assert_eq!(report["fatalError"], false);
    let asserts = report["asserts"].as_array().expect("报告应含 asserts 数组");
    assert_eq!(asserts.len(), 2);
    assert_eq!(asserts[0]["kind"], "noErrors");
    assert_eq!(asserts[0]["passed"], true);
    assert_eq!(asserts[1]["kind"], "creepCountAtLeast");
    assert_eq!(asserts[1]["passed"], false);
    // 人类可读报告转 stderr
    assert!(stderr.contains("模拟报告"), "人类报告应在 stderr:\n{stderr}");
}

/// 意图录制 → 回放保真度: 回放 (纯引擎, 零 JS) 的最终世界状态哈希必须与原运行一致。
#[test]
fn intent_record_and_replay_reproduce_identical_world() {
    let bin = env!("CARGO_BIN_EXE_screeps-sim");
    let scenario_path = tmp_path("replay-scenario.json");
    let bundle_path = tmp_path("replay-bundle.js");
    let record_path = tmp_path("replay-intents.jsonl");
    write_file(
        &scenario_path,
        r#"{
            "name": "replay-test",
            "seed": 3,
            "ticks": 30,
            "rooms": [{
                "name": "W5N8",
                "terrain": { "kind": "flat" },
                "sources": { "positions": [[18, 22]] },
                "controller": { "level": 1, "pos": [28, 27] },
                "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }],
                "creeps": [{ "name": "w", "body": ["work", "carry", "move"], "pos": [19, 22] }]
            }]
        }"#,
    );
    // 采集 + 移动 + 回填 spawn: 产生稳定的意图流
    write_file(
        &bundle_path,
        r#"exports.loop = function() {
            const creep = Object.values(Game.creeps)[0];
            if (!creep) return;
            const spawn = creep.room.find(FIND_MY_SPAWNS)[0];
            const src = creep.room.find(FIND_SOURCES)[0];
            if (creep.store.getFreeCapacity() > 0) {
                if (creep.harvest(src) === ERR_NOT_IN_RANGE) creep.moveTo(src);
            } else if (creep.transfer(spawn, RESOURCE_ENERGY) === ERR_NOT_IN_RANGE) {
                creep.moveTo(spawn);
            }
        };"#,
    );
    let run = |extra: &[&str], report: &std::path::Path| -> serde_json::Value {
        let out = Command::new(bin)
            .arg("--scenario").arg(&scenario_path)
            .args(extra)
            .arg("--json-report").arg(report)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "运行失败\nstderr: {stderr}");
        serde_json::from_str(&std::fs::read_to_string(report).unwrap()).unwrap_or_else(|e| {
            panic!("报告解析失败 {e}\nstdout: {stdout}");
        })
    };
    let r1 = run(&["--bundle", bundle_path.to_str().unwrap(), "--record-intents", record_path.to_str().unwrap()], &tmp_path("replay-r1.json"));
    // 回放: 不传 --bundle (纯引擎, 无 V8)
    let r2 = run(&["--replay", record_path.to_str().unwrap()], &tmp_path("replay-r2.json"));
    assert_eq!(r1["stateHash"], r2["stateHash"], "回放的世界状态哈希必须与原运行一致");
    assert_eq!(r1["ticks"], r2["ticks"]);
    assert!(std::fs::read_to_string(&record_path).unwrap().lines().count() >= 25, "录制文件应包含每 tick 一行");
}

/// tickLimit 场景覆盖值生效: 覆盖为 10000 也不越过官方 500 上限的语义由公式保证,
/// 这里验证覆盖值本身被采用 (Game.cpu.tickLimit 透传由单元测试覆盖)。
#[test]
fn scenario_tick_limit_override_accepted() {
    // 覆盖值为合法正数: 加载不应失败
    let scenario = scenario_json(r#"{"limit": 20, "bucket": 500, "tick_limit": 300}"#, 1, r#""#);
    let s = screeps_sim::engine::scenario::Scenario::from_json(&scenario).unwrap();
    assert_eq!(s.cpu_tick_limit(), 300.0);
    assert_eq!(s.cpu_tick_limit_override(), Some(300.0));
    // 未覆盖时按官方公式 min(500, max(0,bucket) + limit)
    let s2 = screeps_sim::engine::scenario::Scenario::from_json(&scenario_json(
        r#"{"limit": 100, "bucket": 50}"#,
        1,
        r#""#,
    ))
    .unwrap();
    assert_eq!(s2.cpu_tick_limit(), 150.0);
}
