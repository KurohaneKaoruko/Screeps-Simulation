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
  Source 再生（低于容量起 300t 计时）、RCL 升降级、道路/容器/rampart 衰减、掉落能量 `⌈n/1000⌉` 衰减、
  墓碑、GCL、出口格即时越房、spawn 自动回能（<300 时 +1/t）、`renewCreep`/`recycleCreep`、
  `tower` 攻击/治疗/维修（按距离衰减功率）与 `link` 传输（3% 损耗 + 1t 冷却）。
  同 tick 的多个意图按官方 `creepActions` 顺序结算（drop → transfer → withdraw → pickup →
  harvest → move → repair → build → upgradeController），并套用官方 `checkPriorities` 互斥表；
  掉落资源先装入同格 `hits > 0` 的容器，装不下的部分才落地；单位死亡按 `dropRate × 剩余寿命比例`
  回收身体资源（回收为 100%）、携带资源全额，先入容器再进墓碑；超出 RCL 上限的
  spawn/extension 被标记为 `off` 并排除出能量统计。
- **多房间**：移动仲裁按房间独立（官方 `processor.js` 每房间各自 `check`），跨房移动不再互相取消；
  `rate1`（交换对象）优先于同格第三方抢位。
- **运行框架**：CPU 计量含官方意图计价（每个「对象 + 意图」组合首次提交 0.2 CPU，重复不重复计），
  `Game.cpu.tickLimit = min(500, limit + bucket)`；`cpu.enforce` 打开后由看门狗线程在 tickLimit
  到期时中断 V8（官方 "Script execution timed out ungracefully"：整 tick 丢弃、`cpuUsed = tickLimit`）；
  `RawMemory` 段跨 tick 持久（≤10 段、0..99、≤100KB，未重新激活即停用）；`Memory` 超过 2MB 拒绝保存；
  可选 `globalResetEvery` 定期重建 isolate 模拟全局重置（全局变量丢失、Memory 与段保留）。
- **JS shim** (`src/js/screeps.js`)：官方 runtime 语义的 API 层 —— 全局常量（对齐官方
  `constants.js`，含 `CONTROLLER_STRUCTURES` / `EVENT_*` / `TOWER_*` / `ROAD_*` 等）、
  类体系（可被 `Creep.prototype.move` 等原型补丁正常包装）、意图提交与返回码校验、
  `Store`（Proxy 实现，只枚举非零资源、缺失资源返回 0）、
  `PathFinder`（多房 A*，CostMatrix 列优先 `x*50+y` / clamp 0..255 / `serialize` 与官方一致）、
  `moveTo`/`findPath`（reusePath=5 缓存、plainCost 2 / swampCost 10 + 结构成本矩阵）、
  路径序列化 `"XXYY" + 方向字符`、`Room.serializePath`/`deserializePath`、
  完整 lodash 3.10.1 作为全局 `_`（见 `src/js/lodash.js`）、no-op RoomVisual。
- **快照模型**：与真实游戏一致 —— 每 tick 重建 `Game`，bot 读到的永远是 tick 开始时的快照，
  动作结果下一 tick 才可见。可见性由「带 `user` 的对象」决定（仅 creep / controller /
  OwnedStructure / 自己的工地使房间可见），`Game.structures` 只含自己的 OwnedStructure。

## 已知近似（与官方的差异）

| 项 | 说明 |
|---|---|
| PathFinder 跨房计价 | 出口格与对侧入口格按两步计价（官方为同节点），宏观行为一致 |
| 有限世界 | 场景未定义相邻房间时，出口格上的 creep 停留原地（官方无限世界必穿越） |
| 战斗/矿物/市场/Power | 未实现（经济阶段不需要），相关 API 为安全桩：`attack`/`heal`/`dismantle`/`claimController` 等会抛出明确的"未实现"错误，而不是静默失败或 `TypeError` |
| 地形生成 | `generated` 为噪声近似，不复刻官方地形算法；可用 `rows`/`raw` 显式给定真实地形 |
| 事件日志结构 | 已实现 `EVENT_HARVEST/BUILD/REPAIR/UPGRADE_CONTROLLER/TRANSFER/OBJECT_DESTROYED/EXIT`；`objectId` 取发起动作的对象，目标 id 放在 `data.targetId`/`data.sourceId` |
| CPU 墙钟耗时 | `usedTime` 只计 `loop()` 墙钟 + 意图计价，不含 Memory 解析/序列化；跨机器不可复现（P95 断言请留足余量） |
| CPU 中断的意图 | 看门狗中断后该 tick 的意图全部丢弃（官方 runtime 的 `intentsList` 只在 `game.run()` 正常返回后回传），仅保留中断前已刷出的 `console` 输出便于排查 |
| 全局重置触发条件 | 官方按 isolate 堆占用触发；这里用场景的 `globalResetEvery` 固定周期模拟（语义一致：全局变量丢失、Memory 保留） |
| 内存段校验失败的后果 | 官方在 tick 末尾 `throw` 使整个 tick 结果作废；这里改为记入 `errors` 并丢弃该次段写入，其余 tick 结果照常提交 |

## 使用

```bash
# 构建 (Cargo.toml 在仓库根目录; 首次会自动下载 V8 预编译库, 若失败见下方说明)
cargo build --release

# 先构建 bot bundle
npm run build

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

CLI 选项：

| 选项 | 说明 |
|---|---|
| `-s, --scenario <json>` | 场景文件（必填） |
| `-b, --bundle <js>` | bot bundle，默认 `../dist/main.js` |
| `-t, --ticks <n>` | 覆盖场景里的 tick 数 |
| `--csv <path>` | 逐 tick 指标 CSV |
| `--json-report <path>` | 汇总 JSON 报告 |
| `-q, --quiet` | 只输出报告 |
| `-v, --verbose` | 打印全部 `console.log` |
| `--dump-memory <path>` | 落盘最终 Memory |
| `--render-terrain <room>` | 打印指定房间的 ASCII 地形与墙/沼泽占比 |
| `--no-seed-random` | 关闭 `Math.random` 播种（默认播种，seed 取自场景 `seed`） |

### V8 预编译库下载失败时

rusty_v8 的构建脚本偶尔无法跟随 GitHub 302。此时手动下载对应平台（linux/macos/windows、
x86_64/aarch64）的归档，并用 `RUSTY_V8_ARCHIVE` 指向本地文件即可：

```bash
# 例: 手动下载后指定本地归档 (推荐做法, 无需猜测 cargo 缓存的文件名)
curl -L -o /tmp/librusty_v8.a.gz \
  https://github.com/denoland/rusty_v8/releases/download/v150.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
export RUSTY_V8_ARCHIVE=/tmp/librusty_v8.a.gz
cargo build --release
```

## 场景格式

```jsonc
{
  "name": "boot-from-scratch",
  "seed": 42,                 // 确定性随机 (地形/Math.random); seed 为 0 表示不播种
  "ticks": 20000,
  "cpu": {
    "limit": 100,             // 每 tick CPU 上限 (ms)
    "bucket": 10000,          // 初始 bucket
    "tick_limit": 500,        // 覆盖 tickLimit; 默认 min(500, bucket + limit)
    "enforce": false          // true 时超 tickLimit 由看门狗强制中断当 tick (默认仅计量)
  },
  "globalResetEvery": 0,      // >0 时每 N tick 重建 isolate 做一次全局重置; 省略/0 表示不重置
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

场景在构建世界前会做一次校验（`Scenario::validate`），错误信息带房间与字段定位：
房名格式（`W5N8`/`E12S3`）、房间重名、坐标 0..49、控制器等级 0..8、单位重名、
部件名与部件数上限（50）、结构类型、`terrain.raw` 长度 2500、`ticks > 0`、
`cpu.limit > 0`、`globalResetEvery > 0`。

断言类型：`rclAtLeast` / `rclReachedBy` / `creepCountAtLeast` / `cpuAvgBelow` /
`cpuP95Below` / `noErrors`。

`sources` 字段：`{"count": 2, "energy": 3000}` 在中线两侧的候选区内就近搜索非墙格放置
（不是严格对称），或 `{"positions": [[x,y],...]}` 显式指定。

### 机制回归测试

```bash
cargo test
```

| 测试文件 | 覆盖 |
|---|---|
| `tests/engine_rules.rs` | 用受控意图直接驱动引擎（不经 V8）：掉落衰减、Source 再生时序、疲劳步频、出口格越房与弹回、RCL 升级/降级数值、能量转移容量、容器承接掉落、部分拾取、意图顺序与互斥、`off` 停用、GCL 公式、事件日志；以及多房间移动隔离、交换对象优先、可见性判定、墙上路造价 ×150、容器首次衰减 +100、rampart 衰减与 hitsMax、建造障碍、拆除工地掉落、`renewCreep`/`recycleCreep`、`tower.repair`、`link.transferEnergy` |
| `tests/shim_apis.rs` | 微型 bundle 驱动 V8：PathFinder 跨房、地图 API、`moveTo` 端到端闭环（含 Memory 往返与 `_move` 缓存） |
| `tests/cpu_watchdog.rs` | CPU 看门狗中断与 isolate 恢复、超时 tick 的丢弃语义、未超时不误报、意图计价 0.2 CPU/组合 |
| `tests/memory_segments.rs` | `RawMemory` 段跨 tick 持久、需每 tick 重新激活、ID/数量/类型/长度校验、`RawMemory.set` 语义 |
| `tests/framework.rs` | 全局重置（全局变量丢失、Memory 与段保留）、场景加载校验的正/反用例 |

### 复现基准

`scenarios/*.json` 里的 `asserts`（如 `cpuP95Below`）是**基线数值**：CPU 计量口径调整
（意图计价、`usedTime` 取整）后，这些阈值需要按新口径重新标定。

## 目录

```
src/
  engine/        结算引擎 (纯 Rust)
    constants.rs 官方常量表
    objects.rs   世界对象模型
    world.rs     World/Room + 可见性/能量聚合/事件日志
    terrain.rs   地形解析与生成
    movement.rs  移动仲裁与疲劳 (movement.js 移植)
    intents.rs   意图结算 (creep/spawn/room intents 移植)
    tick.rs      Tick 推进顺序 (processor.js 对齐)
    snapshot.rs  可见世界 → JSON 快照
    scenario.rs  场景定义与加载
    metrics.rs   指标与断言
  js/screeps.js  Screeps API shim (bot 的直接运行环境)
  js/lodash.js   lodash 3.10.1 (MIT), 由宿主注入为全局 `_`
  runtime.rs     deno_core 宿主 (op 边界 + CPU 看门狗 + 内存段/全局重置)
  main.rs        CLI
scenarios/       场景文件
tests/           回归测试 (engine_rules / shim_apis / cpu_watchdog / memory_segments / framework)
```
