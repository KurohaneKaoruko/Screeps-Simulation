//! V8 宿主: 装载 screeps.js shim + bot bundle, 每 tick 通过 op 双向传递快照/内存/意图。
//! 不手写 v8 scope —— JS↔Rust 数据边界全部走 deno op, 由 deno_core 负责序列化。

use deno_core::{extension, op2, JsRuntime, RuntimeOptions};
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

#[derive(Default)]
struct TickIo {
    snapshot: String,
    memory: String,
    result: Option<String>,
}

thread_local! {
    static TICK_IO: RefCell<TickIo> = RefCell::new(TickIo::default());
}

/// 高精度计时 (ms, 运行时零点起) — Game.cpu.getUsed 的底层
#[op2(fast)]
fn op_sim_now() -> f32 {
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs_f32() * 1000.0
}

#[op2]
#[string]
fn op_sim_get_snapshot() -> String {
    TICK_IO.with(|t| t.borrow().snapshot.clone())
}

#[op2]
#[string]
fn op_sim_get_memory() -> String {
    TICK_IO.with(|t| t.borrow().memory.clone())
}

#[op2(fast)]
fn op_sim_result(#[string] result: String) {
    TICK_IO.with(|t| t.borrow_mut().result = Some(result));
}

extension!(
    nexus_sim,
    ops = [op_sim_now, op_sim_get_snapshot, op_sim_get_memory, op_sim_result]
);

pub struct TickResult {
    pub intents_json: String,
    pub memory: String,
    pub cpu_used: f64,
    pub errors: Vec<String>,
    pub console: Vec<String>,
}

pub struct BotRuntime {
    runtime: JsRuntime,
}

impl BotRuntime {
    /// 创建 V8 运行时: 先注入 lodash, 再装 shim, 最后以 CJS 方式装载 bundle。
    /// `seed == 0` 表示不播种 (Math.random 保持 V8 原生随机)。
    pub fn new(bundle_code: &str, seed: u64) -> Result<BotRuntime, String> {
        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![nexus_sim::init()],
            ..Default::default()
        });
        // 官方 runtime 把完整 lodash 3.10.1 注入为全局 `_` (E/game/game.js:475-477)
        let lodash = include_str!("js/lodash.js");
        let mut lodash_boot = String::with_capacity(lodash.len() + 160);
        lodash_boot.push_str("(function() { var module = { exports: {} }; var exports = module.exports;\n");
        lodash_boot.push_str(lodash);
        lodash_boot.push_str("\nglobalThis._ = module.exports; })()");
        runtime
            .execute_script("lodash.js", lodash_boot)
            .map_err(|e| format!("lodash 装载失败: {e}"))?;
        let shim = include_str!("js/screeps.js");
        runtime
            .execute_script("screeps.js", shim)
            .map_err(|e| format!("shim 装载失败: {e}"))?;
        // 播种必须在 bundle 之前: 模块顶层的 Math.random 也要确定性 (官方先建沙箱再跑 bundle)
        let seed_call = if seed == 0 {
            String::new()
        } else {
            format!("__seedRandom({seed});")
        };
        let bootstrap = format!(
            "(function() {{ {seed_call} var err = __bootstrapBundle({}); if (err) throw new Error(err); }})()",
            json_string(bundle_code)
        );
        runtime
            .execute_script("bundle.js", bootstrap)
            .map_err(|e| format!("bundle 装载失败: {e}"))?;
        Ok(BotRuntime { runtime })
    }

    /// 运行一个游戏 tick
    pub fn run_tick(&mut self, snapshot_json: &str, memory_json: &str) -> Result<TickResult, String> {
        TICK_IO.with(|t| {
            let mut io = t.borrow_mut();
            io.snapshot = snapshot_json.to_string();
            io.memory = memory_json.to_string();
            io.result = None;
        });
        self.runtime
            .execute_script("tick.js", "__runTick()")
            .map_err(|e| format!("__runTick 执行失败: {e}"))?;
        let raw = TICK_IO
            .with(|t| t.borrow().result.clone())
            .ok_or_else(|| "__runTick 未提交结果".to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("tick 结果解析失败: {e}"))?;
        Ok(TickResult {
            intents_json: value.get("intents").cloned().unwrap_or_default().to_string(),
            memory: value
                .get("memory")
                .and_then(|m| m.as_str())
                .unwrap_or("{}")
                .to_string(),
            cpu_used: value.get("cpuUsed").and_then(|c| c.as_f64()).unwrap_or(0.0),
            errors: value
                .get("errors")
                .and_then(|e| e.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            console: value
                .get("console")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        })
    }
}

/// 字符串安全嵌入 JS 源 (bundle 代码)
fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}
