//! CLI 级集成测试: bucket 耗尽跳过、noErrors 断言、退出码语义。
//! 通过 CARGO_BIN_EXE 直接驱动二进制, 验证 main.rs 主循环的官方 bucket 行为。

use std::io::Write;
use std::process::Command;

fn tmp_path(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("nexus-sim-test-{}-{name}", std::process::id()));
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
    let bin = env!("CARGO_BIN_EXE_nexus-sim");
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
    let bin = env!("CARGO_BIN_EXE_nexus-sim");
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

/// tickLimit 场景覆盖值生效: 覆盖为 10000 也不越过官方 500 上限的语义由公式保证,
/// 这里验证覆盖值本身被采用 (Game.cpu.tickLimit 透传由单元测试覆盖)。
#[test]
fn scenario_tick_limit_override_accepted() {
    // 覆盖值为合法正数: 加载不应失败
    let scenario = scenario_json(r#"{"limit": 20, "bucket": 500, "tick_limit": 300}"#, 1, r#""#);
    let s = nexus_sim::engine::scenario::Scenario::from_json(&scenario).unwrap();
    assert_eq!(s.cpu_tick_limit(), 300.0);
    assert_eq!(s.cpu_tick_limit_override(), Some(300.0));
    // 未覆盖时按官方公式 min(500, max(0,bucket) + limit)
    let s2 = nexus_sim::engine::scenario::Scenario::from_json(&scenario_json(
        r#"{"limit": 100, "bucket": 50}"#,
        1,
        r#""#,
    ))
    .unwrap();
    assert_eq!(s2.cpu_tick_limit(), 150.0);
}
