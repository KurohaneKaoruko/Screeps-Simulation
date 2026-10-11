# Screeps-Simulation

本地高速模拟器：Rust 结算引擎 + 内嵌 V8（deno_core）运行 bot 的真实 bundle。
用于在开发期以数千 tick/秒的速度验证 bot 行为，替代"上传到服务器等 4 万 tick"的验证回路。

## 架构

```
┌─────────────── 每 tick 循环 ───────────────┐
│  Rust 引擎 (src/engine/)                    │
│   世界状态 → JSON 快照 ──→ op ──→ JS shim   │
│                                        │    │
│   V8 (deno_core)  src/js/screeps.js    │    │
│   Game/Memory/PathFinder 全套 API       │    │
│   bot bundle (dist/main.js) loop()     │    │
│                                        │    │
│   意图数组 ←──── op ←─── 收集 intents   │    │
│   Rust 按官方引擎语义结算 → 推进 tick    │    │
└────────────────────────────────────────────┘
```

- **结算引擎** (`src/engine/`)：纯 Rust，不依赖 JS。数值与结算顺序对齐官方引擎
  （常量来自 docs.screeps.com，结算逻辑对照 `screeps/engine` 源码移植）：
  疲劳移动与碰撞仲裁、采集/搬运/建造/维修/升级、孵化（`3t × 部件数`、就近扣能、出生落位）、
  Source 再生（低于容量起 300t 计时）、RCL 升降级、道路/容器衰减、掉落能量 `⌈n/1000⌉` 衰减、
  墓碑、GCL、出口格即时越房、spawn 自动回能（<300 时 +1/t）。
- **JS shim** (`src/js/screeps.js`)：官方 runtime 语义的 API 层 —— 全局常量、类体系
  （可被 `Creep.prototype.move` 等原型补丁正常包装）、意图提交与返回码校验、
  `PathFinder`（多房 A*，CostMatrix 语义与官方一致）、`moveTo`/`findPath`
  （reusePath=5 缓存、plainCost 2 / swampCost 10 + 结构成本矩阵）、no-op RoomVisual。
- **快照模型**：与真实游戏一致 —— 每 tick 重建 `Game`，bot 读到的永远是 tick 开始时的快照，
  动作结果下一 tick 才可见。

## 已知近似（与官方的差异）

| 项 | 说明 |
|---|---|
| PathFinder 跨房计价 | 出口格与对侧入口格按两步计价（官方为同节点），宏观行为一致 |
| moveTo 默认 ignoreCreeps | 按官方文档语义取 `true`（引擎源码中按 falsy 处理会绕开 creep） |
| 有限世界 | 场景未定义相邻房间时，出口格上的 creep 停留原地（官方无限世界必穿越） |
| 战斗/矿物/市场/Power | 未实现（经济阶段不需要），相关 API 为安全桩 |
| 地形生成 | `generated` 为噪声近似，不复刻官方地形算法；可用 `rows`/`raw` 显式给定真实地形 |

## 使用

```bash
# 构建 (首次会自动下载 V8 预编译库; 若下载失败见下方说明)
cd sim && cargo build --release

# 先构建 bot bundle
cd .. && npm run build

# 跑场景 (dist/main.js 是真实上传的产物)
./target/release/nexus-sim \
    --scenario scenarios/boot-from-scratch.json \
    --bundle ../dist/main.js \
    --ticks 20000 \
    --csv /tmp/ticks.csv \
    --json-report /tmp/report.json
```

输出：RCL 时间线、能量产/耗、孵化/死亡、CPU 均值/P95/P99、tick/s、Memory 峰值，
场景断言（`asserts`）失败时退出码非 0，可直接接入回归。

### V8 预编译库下载失败时

rusty_v8 构建脚本偶尔无法跟随 GitHub 302。手动下载并放入 cargo 缓存：

```bash
curl -L -o /tmp/librusty_v8.a.gz \
  https://github.com/denoland/rusty_v8/releases/download/v150.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
cp /tmp/librusty_v8.a.gz ~/.cargo/.rusty_v8/https___github_com_denoland_rusty_v8_releases_download_v150_4_0_librusty_v8_simdutf_release_x86_64_unknown_linux_gnu_a_gz
```

## 场景格式

```jsonc
{
  "name": "boot-from-scratch",
  "seed": 42,                 // 确定性随机 (地形/Math.random)
  "ticks": 20000,
  "cpu": { "limit": 100, "bucket": 10000 },   // CPU 计量与 bucket 演化
  "always_visible": false,    // 调试: 全房间常可见
  "deterministic_random": true,
  "rooms": [{
    "name": "W5N8",
    "terrain": { "kind": "flat" },            // flat | generated{seed} | rows | raw
    "sources": { "count": 2, "energy": 3000 },
    "controller": { "level": 1, "pos": [28, 27] },
    "structures": [{ "type": "spawn", "pos": [25, 25], "energy": 300 }],
    "creeps": [{ "name": "W1", "body": ["work","carry","move"], "pos": [26, 25] }]
  }],
  "asserts": [
    { "kind": "rclAtLeast", "value": 3 },
    { "kind": "rclReachedBy", "value": 4, "tick": 40000 },
    { "kind": "cpuP95Below", "value": 20 },
    { "kind": "noErrors" }
  ]
}
```

断言类型：`rclAtLeast` / `rclReachedBy` / `creepCountAtLeast` / `cpuAvgBelow` /
`cpuP95Below` / `noErrors`。指标对齐 docs/13 §13.3 回归基准。

`sources` 字段：`{"count": 2, "energy": 3000}` 自动对称放置，或 `{"positions": [[x,y],...]}` 显式指定。

### 机制回归测试

```bash
cargo test --release
```

`tests/engine_rules.rs` 用受控意图直接驱动引擎（不经 V8），锁定掉落衰减、Source 再生时序、
疲劳步频、出口格越房与弹回、RCL 升级/降级数值；`tests/shim_apis.rs` 用微型 bundle 驱动 V8 验证
PathFinder 跨房、地图 API、moveTo 端到端闭环。

## 目录

```
src/
  engine/        结算引擎 (纯 Rust)
    constants.rs 官方常量表
    objects.rs   世界对象模型
    world.rs     World/Room + 可见性/能量聚合
    terrain.rs   地形解析与生成
    movement.rs  移动仲裁与疲劳 (movement.js 移植)
    intents.rs   意图结算 (creep/spawn/room intents 移植)
    tick.rs      Tick 推进顺序 (processor.js 对齐)
    snapshot.rs  可见世界 → JSON 快照
    scenario.rs  场景定义与加载
    metrics.rs   指标与断言
  js/screeps.js  Screeps API shim (bot 的直接运行环境)
  runtime.rs     deno_core 宿主 (op 边界)
  main.rs        CLI
scenarios/       场景文件
```
