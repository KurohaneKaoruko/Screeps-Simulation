//! Screeps World 结算引擎: 数值与结算顺序对齐官方引擎 (docs.screeps.com + screeps/engine 源码核对)。
//! 只关心世界状态与意图, 不涉及 JS 执行 —— JS 宿主由 runtime.rs 提供。

pub mod checkpoint;
pub mod constants;
pub mod cpu;
pub mod intents;
pub mod metrics;
pub mod movement;
pub mod npc;
pub mod objects;
pub mod scenario;
pub mod snapshot;
pub mod terrain;
pub mod tick;
pub mod world;

