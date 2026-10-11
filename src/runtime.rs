//! V8 宿主: 装载 screeps.js shim + bot bundle, 每 tick 通过 op 双向传递快照/内存/意图。
//! 不手写 v8 scope —— JS↔Rust 数据边界全部走 deno op, 由 deno_core 负责序列化。
//!
//! CPU 强制中断 (官方 "Script execution timed out ungracefully"):
//! 看门狗线程在 tickLimit 到期后调用 `terminate_execution`, 中断当 tick 的 JS 执行。
//! 官方 runtime 的意图列表只在 `game.run()` 正常返回后才回传 (driver lib/runtime/runtime.js),
//! 因此中断的 tick 与官方一致: 意图全部丢弃, Memory 改动不落盘, cpuUsed = tickLimit。
//! 唯一例外是控制台输出 —— 它在中断前已节流刷出, 便于排查 bot 卡死 (不影响任何游戏状态)。

use deno_core::{extension, op2, v8, JsRuntime, RuntimeOptions};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

#[derive(Default)]
struct TickIo {
    snapshot: String,
    memory: String,
    /// 运行时数据 (RawMemory 段: 本 tick 激活的段及其内容)
    runtime_data: String,
    result: Option<String>,
    /// 中断前已刷出的控制台输出 (仅用于排查, 不影响游戏状态)
    flushed_console: Vec<String>,
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

/// 控制台节流刷盘 (CPU 中断时仍能看到日志)
#[op2(fast)]
fn op_sim_console(#[string] line: String) {
    TICK_IO.with(|t| t.borrow_mut().flushed_console.push(line));
}

/// 本 tick 的运行时数据 (官方 driver 的 `data.memorySegments` 等)
#[op2]
#[string]
fn op_sim_get_runtime() -> String {
    TICK_IO.with(|t| t.borrow().runtime_data.clone())
}

extension!(
    screeps_sim,
    ops = [
        op_sim_now,
        op_sim_get_snapshot,
        op_sim_get_memory,
        op_sim_get_runtime,
        op_sim_result,
        op_sim_console
    ]
);

pub struct TickResult {
    pub intents_json: String,
    pub memory: String,
    pub cpu_used: f64,
    pub errors: Vec<String>,
    pub console: Vec<String>,
    /// 当 tick 因超出 tickLimit 被强制中断
    pub timed_out: bool,
}

pub struct BotRuntime {
    /// Option 以便全局重置时先销毁旧 isolate 再建新的
    /// (v8 不允许两个 isolate 的句柄作用域同时存在)
    runtime: Option<JsRuntime>,
    /// 跨线程终止句柄 (看门狗用)
    isolate: v8::IsolateHandle,
    /// 内存段存储 (官方 user.memorySegments, 跨 tick 持久; 未激活的段不可读但仍保留)
    segments: std::collections::BTreeMap<u32, String>,
    /// 下一 tick 激活的段 (官方: 不调用 setActiveSegments 即视为全部停用)
    active_segments: Vec<u32>,
    /// 重建 isolate 所需 (全局重置)
    bundle_code: String,
    seed: u64,
}

/// 看门狗: 到点抢占式终止 V8 执行。
/// 用 `claimed` 互斥标记与主线程串行化, 保证"tick 已结束"后不会再发出终止请求。
struct Watchdog {
    finished: Arc<AtomicBool>,
    claimed: Arc<Mutex<bool>>,
    handle: std::thread::JoinHandle<bool>,
}

impl Watchdog {
    fn spawn(isolate: v8::IsolateHandle, timeout_ms: f64) -> Watchdog {
        let finished = Arc::new(AtomicBool::new(false));
        let claimed = Arc::new(Mutex::new(false));
        let (f, c, h) = (finished.clone(), claimed.clone(), isolate);
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs_f64(timeout_ms / 1000.0);
            loop {
                if f.load(Ordering::Relaxed) {
                    return false;
                }
                if Instant::now() >= deadline {
                    let mut claimed = c.lock().unwrap();
                    if *claimed || f.load(Ordering::Relaxed) {
                        return false;
                    }
                    *claimed = true;
                    return h.terminate_execution();
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        });
        Watchdog { finished, claimed, handle }
    }

    /// 结束看门狗; 返回是否真的发出过终止请求
    fn finish(self) -> bool {
        self.finished.store(true, Ordering::Relaxed);
        let claimed = *self.claimed.lock().unwrap();
        let _ = self.handle.join();
        claimed
    }
}

impl BotRuntime {
    /// 创建 V8 运行时: 先注入 lodash, 再装 shim, 最后以 CJS 方式装载 bundle。
    /// `seed == 0` 表示不播种 (Math.random 保持 V8 原生随机)。
    pub fn new(bundle_code: &str, seed: u64) -> Result<BotRuntime, String> {
        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![screeps_sim::init()],
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
        let isolate = runtime.v8_isolate().thread_safe_handle();
        Ok(BotRuntime {
            runtime: Some(runtime),
            isolate,
            segments: Default::default(),
            active_segments: vec![],
            bundle_code: bundle_code.to_string(),
            seed,
        })
    }

    /// 全局重置: 丢弃整个全局作用域并重新装载 shim + bundle (官方 global reset)。
    /// `Memory` / 内存段 / 上一 tick 的激活列表由宿主持有, 因此不受影响 —— 正是官方语义
    /// (全局变量丢失、Memory 保留), 可用来暴露依赖全局变量的 bot。
    pub fn reset_global(&mut self) -> Result<(), String> {
        let bundle_code = self.bundle_code.clone();
        let seed = self.seed;
        let segments = std::mem::take(&mut self.segments);
        let active_segments = std::mem::take(&mut self.active_segments);
        // 先彻底销毁旧 isolate, 再创建新的
        self.runtime = None;
        let mut fresh = BotRuntime::new(&bundle_code, seed)?;
        fresh.segments = segments;
        fresh.active_segments = active_segments;
        *self = fresh;
        Ok(())
    }

    fn rt(&mut self) -> &mut JsRuntime {
        self.runtime.as_mut().expect("isolate 已被销毁")
    }

    /// 运行一个游戏 tick (不限制 CPU)
    pub fn run_tick(&mut self, snapshot_json: &str, memory_json: &str) -> Result<TickResult, String> {
        self.run_tick_limited(snapshot_json, memory_json, None)
    }

    /// 运行一个游戏 tick; `timeout_ms` 为 Some 时启用 CPU 看门狗 (官方 tickLimit)。
    pub fn run_tick_limited(
        &mut self,
        snapshot_json: &str,
        memory_json: &str,
        timeout_ms: Option<f64>,
    ) -> Result<TickResult, String> {
        TICK_IO.with(|t| {
            let mut io = t.borrow_mut();
            io.snapshot = snapshot_json.to_string();
            io.memory = memory_json.to_string();
            io.runtime_data = self.runtime_data_json();
            io.result = None;
            io.flushed_console.clear();
        });

        let watchdog = timeout_ms
            .filter(|ms| *ms > 0.0)
            .map(|ms| Watchdog::spawn(self.isolate.clone(), ms));

        let exec = self.rt().execute_script("tick.js", "__runTick()");

        let claimed = watchdog.map(|w| w.finish()).unwrap_or(false);
        // 终止标志必须清除, 否则后续 tick 会立刻被中断
        self.rt().v8_isolate().cancel_terminate_execution();

        let terminated = claimed
            && exec
                .as_ref()
                .err()
                .map(|e| e.to_string().contains("execution terminated"))
                .unwrap_or(false);

        if terminated {
            // 官方: 中断的 tick 整 tick 丢弃 (意图列表随 isolate 一起丢失), Memory 改动不落盘
            let console = TICK_IO.with(|t| {
                t.borrow().flushed_console.iter().take(50).cloned().collect::<Vec<_>>()
            });
            return Ok(TickResult {
                intents_json: "[]".to_string(),
                memory: memory_json.to_string(),
                cpu_used: timeout_ms.unwrap_or(0.0),
                errors: vec!["Script execution timed out ungracefully".to_string()],
                console,
                timed_out: true,
            });
        }

        exec.map_err(|e| format!("__runTick 执行失败: {e}"))?;
        let raw = TICK_IO
            .with(|t| t.borrow().result.clone())
            .ok_or_else(|| "__runTick 未提交结果".to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("tick 结果解析失败: {e}"))?;
        self.apply_segments(&value);
        Ok(TickResult {
            intents_json: value.get("intents").cloned().unwrap_or_default().to_string(),
            // 官方 saveUserMemory 超过 2MB 会拒绝保存 → 保留上一 tick 的 Memory
            memory: value
                .get("memory")
                .and_then(|m| m.as_str())
                .map(String::from)
                .unwrap_or_else(|| memory_json.to_string()),
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
            timed_out: false,
        })
    }

    /// 交给 shim 的运行时数据: 本 tick 激活的段及其内容 (官方 driver `data.memorySegments`)
    fn runtime_data_json(&self) -> String {
        let mut segments = serde_json::Map::new();
        for id in &self.active_segments {
            if let Some(v) = self.segments.get(id) {
                segments.insert(id.to_string(), serde_json::Value::String(v.clone()));
            }
        }
        serde_json::json!({
            "activeSegments": self.active_segments,
            "segments": segments,
        })
        .to_string()
    }

    /// 应用 shim 回传的段写入与下一 tick 激活列表 (官方 outMessage.memorySegments / activeSegments)。
    /// 官方在 isolate 被中断时拿不到 outMessage, 因此超时 tick 不写段 (此处也不调用)。
    fn apply_segments(&mut self, value: &serde_json::Value) {
        if let Some(segs) = value.get("memorySegments").and_then(|v| v.as_object()) {
            for (k, v) in segs {
                if let (Ok(id), Some(s)) = (k.parse::<u32>(), v.as_str()) {
                    self.segments.insert(id, s.to_string());
                }
            }
        }
        self.active_segments = value
            .get("activeSegments")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
    }

    /// 已存储的内存段 (测试/排查用)
    pub fn segments(&self) -> &std::collections::BTreeMap<u32, String> {
        &self.segments
    }
}

/// 字符串安全嵌入 JS 源 (bundle 代码)
fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}
