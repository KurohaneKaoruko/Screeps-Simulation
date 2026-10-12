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
  **战斗链路**：近战（`ATTACK_POWER=30`/部件 + 目标 hit-back 反击）、远程（`RANGED_ATTACK_POWER=10`/部件、
  3 格）、群体攻击（距离衰减 `{1,2,3} = {1.0, 0.4, 0.1}`）、治疗（近战 12 / 远程 4）、
  拆除（`DISMANTLE_POWER=50`、返还 `⌊damage×0.005⌋`）、控制器操作（`claim` / `reserve`（上限 5000）/
  `sign` / `attackController`（降级计时提前 300 + 升级封锁 1000t））；
  rampart 重定向、伤害累积至 tick 末统一应用、`_recalc-body` 从身体头部开始损坏（尾部先满血）、
  容量缩小自动掉落超容资源。
  **工业链路**：采矿（`Creep.harvest(mineral)` 需同格自有且激活的 extractor，`HARVEST_MINERAL_POWER × WORK`，
  extractor 冷却 5（官方 `_cooldown`，每 6 tick 一采）；矿物枯竭后 `MINERAL_REGEN_TIME` 按密度回满并按官方概率换密度）、
  部件强化（`body[i].boost`：采集/拆除/攻击/远程/群攻/治疗/CARRY 容量/MOVE 疲劳按 `BOOSTS` 倍率，
  build/repair/upgradeController 的强化额外量与官方 `boostedParts` 截断一致且能量按未强化量扣，TOUGH 减伤、
  负重折算、死亡返还化合物、renew 清除强化）、实验室（`runReaction`/`reverseReaction`/`boostCreep`/`unboostCreep`，
  `REACTION_TIME` 冷却、lab 只持有一种矿物、unboost 返还 15/件 + 反应链冷却）、终端（`send` 在 tick 末的全局阶段执行：
  目标仓空间截断、能量费 `⌈amount × (1 − e^(−距离/30))⌉`、冷却 10；`Game.market.calcTransactionCost` 与
  `incoming/outgoingTransactions`）、观察者（`observeRoom` 的房间下一 tick 可见）、`PowerSpawn.processPower`
  （累计 power → `Game.gpl`）、`Factory.produce`（官方 `COMMODITIES` 配方）。所有自有结构按官方
  `checkStructureAgainstController` 判定激活（离控制器近者优先占名额），terminal/factory 容量随 RCL；
  掉落资源按 `resourceType` 分堆（`LOOK_RESOURCES` / `FIND_DROPPED_RESOURCES` 返回任意资源）。
  同 tick 的多个意图按官方 `creepActions` 顺序结算（drop → transfer → withdraw → pickup →
  heal → rangedHeal → dismantle → attack → harvest → move → repair → build → rangedMassAttack →
  rangedAttack → say → suicide → claimController → upgradeController → reserveController →
  attackController → signController），并套用官方 `checkPriorities` 互斥表；
  掉落资源先装入同格 `hits > 0` 的容器，装不下的部分才落地；单位死亡按 `dropRate × 剩余寿命比例`
  回收身体资源（回收为 100%）、携带资源全额，先入容器再进墓碑；超出 RCL 上限的
  spawn/extension 被标记为 `off` 并排除出能量统计。
- **多房间**：移动仲裁按房间独立（官方 `processor.js` 每房间各自 `check`），跨房移动不再互相取消；
  `rate1`（交换对象）优先于同格第三方抢位。
- **运行框架**：CPU 计量含官方意图计价（每个「对象 + 意图」组合首次提交 0.2 CPU，重复不重复计），
  `Game.cpu.tickLimit = min(500, max(0, bucket) + limit)`、`Game.cpu.bucket` 显示 `max(0, raw)`；
  bucket 按官方 `raw = min(10000, raw + limit − ceil(cpuUsed))` 结算，**可以为负** —— 为负时跳过该 tick
  的用户执行（官方 "Script execution has been terminated: CPU bucket is empty"，世界照常推进，
  跳过期间每 tick 回能 `limit`、下限 `-2×limit`）；`cpu.enforce` 打开后由看门狗线程在 tickLimit
  到期时中断 V8（官方 "Script execution timed out ungracefully"：整 tick 丢弃、`cpuUsed = tickLimit`）；
  `RawMemory` 段跨 tick 持久（≤10 段、0..99、≤100KB，未重新激活即停用）；`Memory` 超过 2MB 拒绝保存；
  可选 `globalResetEvery` 定期重建 isolate 模拟全局重置（全局变量丢失、Memory 与段保留）。
  运行时致命错误会计入错误统计并以退出码 1 结束（不再"带着错误通过断言"）。
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
| 市场订单 / Power Creep / Nuker | 未实现：`Game.market` 的订单/`deal`/`credits` 为 no-op 桩（`terminal.send`、`calcTransactionCost`、转账记录已实现），`Game.powerCreeps` 恒空，`launchNuke` 缺失（bundle 预检会报错） |
| 工业的 Power 效果 | 不模拟任何 `PWR_*` 效果（`PWR_OPERATE_LAB/TERMINAL/FACTORY/POWER`、`PWR_REGEN_MINERAL` 等）；因此**有等级的工厂商品**（`COMMODITIES[x].level > 0`）恒失败（API 返回 `ERR_BUSY`/`ERR_INVALID_TARGET`，与官方无效果时一致），只有 0 级商品可生产；工厂等级只能由场景 `structures[].level` 给定 |
| 矿物换密度的随机数 | 官方处理器用 `Math.random`；这里用场景 `seed` 播种的引擎随机数（splitmix64），概率分布一致、序列不同，同 seed 可复现 |
| 转账记录 | 单玩家：发往自己终端的转账同时出现在 `incomingTransactions` 与 `outgoingTransactions`；只保留最近 100 条，`sender/recipient` 的 username 固定为 `Nexus`（敌方终端无 recipient） |
| 敌方结构的激活 | 结构 `isActive()`（RCL 名额）只对我方结构计算，场景放置的敌方结构恒为激活 |
| 战斗（近似完成） | 近战/远程/群体攻击、治疗、拆除、claim/reserve/sign/attackController 已对齐官方语义（含 hit-back、rampart 重定向、部件从头损坏、伤害 tick 末统一应用、意图互斥表）；未覆盖：Power 效果（`PWR_FORTIFY` 等护盾）、safeMode 拦截、nuker |
| 地形生成 | `generated` 为噪声近似，不复刻官方地形算法；可用 `rows`/`raw` 显式给定真实地形 |
| 事件日志结构 | 已实现 `EVENT_HARVEST/BUILD/REPAIR/UPGRADE_CONTROLLER/TRANSFER/OBJECT_DESTROYED/EXIT/ATTACK/HEAL/ATTACK_CONTROLLER/RESERVE_CONTROLLER`；`objectId` 取发起动作的对象，目标 id 放在 `data.targetId`/`data.sourceId`（采能量用 `sourceId`，采矿按官方用 `targetId`、`amount` 为未截断的采集功率）；lab/terminal/factory/observer/powerSpawn 不产生事件（官方只写 actionLog） |
| CPU 墙钟耗时 | `usedTime` 只计 `loop()` 墙钟 + 意图计价，不含 Memory 解析/序列化；跨机器不可复现（P95 断言请留足余量） |
| CPU 中断的意图 | 看门狗中断后该 tick 的意图全部丢弃（官方 runtime 的 `intentsList` 只在 `game.run()` 正常返回后回传），仅保留中断前已刷出的 `console` 输出便于排查 |
| 全局重置触发条件 | 官方按 isolate 堆占用触发；这里用场景的 `globalResetEvery` 固定周期模拟（语义一致：全局变量丢失、Memory 保留） |
| 内存段校验失败的后果 | 官方在 tick 末尾 `throw` 使整个 tick 结果作废；这里改为记入 `errors` 并丢弃该次段写入，其余 tick 结果照常提交 |

## 快速开始

```bash
# 构建 Rust 引擎 (首次约 1 分钟, 会自动下载 V8 预编译库, 失败见下方说明)
cargo build --release

# 自测 (不依赖真实 bot, 用 examples/fixtures/min-bot.js 陪跑)
npm test
```

## 编写 .sim.ts 模拟测试 (推荐)

像写单元测试一样定义场景 + 断言, runner 逐用例运行并汇总, 失败退出码非 0 —— 可直接进 CI:

```ts
// sim/boot.sim.ts
import { defineSim } from 'screeps-sim'

export default defineSim({
  name: 'boot-from-scratch',
  seed: 42,                       // 确定性随机 (地形 / Math.random)
  ticks: 20000,
  cpu: { limit: 100, bucket: 10000 },
  rooms: [{
    name: 'W5N8',
    terrain: { kind: 'flat' },    // flat | generated{seed} | rows | raw
    sources: { positions: [[18, 22], [32, 30]] },
    controller: { level: 1, pos: [28, 27] },
    mineral: { pos: [38, 38], type: 'H', amount: 30000 },
    structures: [{ type: 'spawn', pos: [25, 25], energy: 300 }],
    creeps: [],
  }],
  asserts: [
    { kind: 'rclReachedBy', value: 2, tick: 5000 },
    { kind: 'noErrors' },
  ],
})
```

```bash
# 跑当前目录下所有 *.sim.ts (自动跳过 node_modules/target/dist 等)
screeps-sim test --bundle dist/main.js

# 指定文件 / 目录 / 通配符
screeps-sim test sim/boot.sim.ts
screeps-sim test sim/                 # 目录递归
screeps-sim test 'sim/**/*.sim.ts'    # 通配符

# 只列出用例 / 快速冒烟 (覆盖 tick 数) / 显示 bot console
screeps-sim test --list
screeps-sim test --bundle dist/main.js --ticks 500
screeps-sim test -v
```

输出形如:

```
sim/boot.sim.ts
  ✓ boot-from-scratch  20000 ticks  ·  36.2s  ·  552 t/s  ·  RCL L1@0 L2@669  ·  cpu p95 3.0ms
  ✗ rcl-sprint  50000 ticks  ·  ...
      · 断言 rclReachedBy: RCL4 达成于 None, 要求 ≤40000

存在失败  ·  1/2 通过  ·  总用时 40.1s
```

规则与约定:

- **一个文件多个用例**: `default` 导出一个, 其余具名导出 (返回值"长得像场景"即被识别);
  `skip: true` 的用例只列出、不运行。
- **bundle 解析顺序**: `--bundle` > 用例内 `bundle` 字段 (相对 `.sim.ts` 文件) >
  项目 `package.json` 的 `"screeps-sim".bundle` > 自动探测 `dist/main.js` 等常见布局。
- **字段风格**: TS 里写 camelCase (`alwaysVisible` / `cpu.tickLimit` / `mineral.type`),
  runner 负责映射到引擎 JSON; 未知字段原样透传, 所以直接写 wire 字段名也可以。
- **类型**: `import { defineSim } from 'screeps-sim'` 自带完整类型提示 (见 `index.d.ts`),
  需要 Node ≥ 22.6 (原生加载 TS); 旧 Node 可改用 `.sim.mjs` / `.sim.js`。
- 完整示例见 `examples/` (smoke 自包含可跑; boot-from-scratch / rcl-sprint 需真实 bundle)。

### 模拟前静态检查 (编译器式诊断)

`screeps-sim check` 与 `test` 的预检会在**引擎启动前**指出能静态发现的问题:

```bash
screeps-sim check                      # 检查所有场景定义 (跳过的用例也检查)
screeps-sim check sim/boot.sim.ts      # 指定文件
screeps-sim check --bundle dist/main.js # 只检查 bot bundle
screeps-sim check --strict             # 警告也计为失败 (CI 用)
```

**场景检查** (`check.mjs`, 错误 → 模拟必失败; 警告 → 可疑但合法):

- 硬错误: 房名格式/重复、坐标越界、controller 等级、部件名/数量、结构类型、
  单位重名(全游戏唯一)、terrain rows/raw 规格、非法 JSON memory、未知断言类型、
  不可能达成的断言 (`rclReachedBy.tick > ticks`) 等
- 警告: RCL 超限结构 (引擎会标 off)、同格多结构、墙上对象、**房间不连通**(房名拼写错误的典型症状)、
  `seed=0` 不可复现、`alwaysVisible` 调试残留、无断言场景等

**bot bundle 检查** (`bundle-check.mjs`, 基于 433 个官方常量 + 模拟器覆盖度表):

- 错误: **常量拼写错误** (`ERR_NOT_IN_RANG` → 运行时 undefined, 比较永远失败)、
  **参数顺序颠倒** (`transfer(RESOURCE_ENERGY, target)` / `spawnCreep('name', body)`)、
  **node 泄漏** (`process.` / `require(` / `Buffer.` —— 官方同样会炸)、
  调用模拟器未实现的方法 (`launchNuke` → TypeError)
- 警告: `setTimeout` 等运行时缺失的 API、`WebAssembly.instantiate` 跨 tick 异步必死、
  **模拟覆盖度** (`Game.market` 订单/deal no-op、powerCreeps 恒空 —— 提示哪些行为不会被模拟)

常量表从 `src/js/screeps.js` 自动提取 (`node runner/gen-constants.mjs`), `npm test` 校验同步。
`test` 运行时自动做两层预检: 场景错误 → 用例秒失败; bundle 错误 → 使用该 bundle 的用例全部秒失败,
警告打印一次 —— 烂代码不再烧几万 tick 才发现。

### 编程 API

```js
import { defineSim, runSim } from 'screeps-sim'
const report = runSim(myScenario, { bundle: 'dist/main.js', ticks: 1000 })
console.log(report.rclTimeline, report.asserts)
```

## 给其他项目安装 (npm 包)

本仓库本身就是一个 npm 包 (`screeps-sim`, 零运行时依赖), 在任意 bot 项目里:

```bash
# 本地路径安装 (开发期推荐, 源码改动即时生效)
npm i -D path/to/Screeps-Simulation

# 或 git 安装
npm i -D github.com:KurohaneKaoruko/Screeps-Simulation
```

bot 项目一次性配置 (`package.json`):

```jsonc
{
  "scripts": {
    "sim": "screeps-sim test",              // npm run sim [-- --bundle ...]
    "sim:ci": "screeps-sim test --ticks 500" // 进 CI, 断言失败即红
  },
  "screeps-sim": {
    "bundle": "dist/main.js",   // 默认 bundle (相对项目根; 不配则自动探测 dist/main.js 等)
    "include": ["sim"]          // 默认扫描范围 (不配则扫当前目录, 跳过 node_modules/target/dist)
  }
}
```

然后写 `sim/*.sim.ts` 即可。两种写法任选:

```ts
// ① 正规写法: import defineSim, 获得完整类型提示 (项目无需任何 TS 配置, Node ≥22.6 原生加载)
import { defineSim } from 'screeps-sim'
export default defineSim({ name: 'boot', rooms: [/* ... */], asserts: [/* ... */] })

// ② 零依赖写法: 不装类型也能跑 —— 纯对象导出 (有 name + rooms 字段即被识别)
export const myCase = { name: 'boot', rooms: [/* ... */] }
```

说明:

- 一个文件可导出**多个用例** (`default` + 任意具名导出), `skip: true` 跳过;
  用 TS 表达式共享房间模板/断言组合, 这是比 JSON 场景强得多的地方。
- 需要 Node ≥ 22.6 (原生 TS 加载); CommonJS 项目也可用 —— Node 22+ 会按语法自动把
  含 `import/export` 的 `.ts` 识别为 ESM (极旧环境可改 `.sim.mts`)。
- 首次运行会自动 `cargo build --release` (需要 Rust 工具链); 已构建好的二进制可用
  `SCREEPS_SIM_BIN` 环境变量指定。

## 直跑单个场景 (CLI)

不经 runner, 直接驱动引擎 (调试 / 临时实验):

```bash
./target/release/screeps-sim \
    --scenario scenarios/boot-from-scratch.json \
    --bundle dist/main.js \
    --ticks 20000 \
    --csv /tmp/ticks.csv \
    --json-report /tmp/report.json
```

输出：RCL 时间线、能量产/耗、孵化/死亡、CPU 均值/P95/P99、tick/s、Memory 峰值，
场景断言（`asserts`）失败时退出码非 0，可直接接入回归。

CLI 选项：

| 选项 | 说明 |
|---|---|
| `-s, --scenario <json\|->` | 场景文件 (必填); `-` 从 stdin 读 (runner 的管道协议) |
| `-b, --bundle <js>` | bot bundle，默认 `dist/main.js` |
| `-t, --ticks <n>` | 覆盖场景里的 tick 数 |
| `--csv <path>` | 逐 tick 指标 CSV |
| `--json-report <path\|->` | 汇总 JSON 报告 (含断言结果/分相耗时/状态哈希); `-` 写 stdout, 人类可读报告转 stderr |
| `--record-intents <path>` | 每 tick 意图流落盘 JSONL (A/B 对比、回放原料) |
| `--replay <path>` | 纯引擎回放意图流, **零 JS/零 V8** (~11 万 t/s), 不需要 `--bundle` |
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

### rustc 已知问题

本仓库当前工具链 rustc 1.95.0（发行版源码 tarball 构建）在 `--release`（opt-level 3）下会把
`objects.rs::recalc_body` 中形如 `if hits > 100 { 100 } else { hits.max(0) as u32 }` 的表达式
错误编译成不封顶的赋值（debug / opt-level 0 均正确）。已改用 `hits.clamp(0, 100)` 规避；
修改该函数时请保留 clamp 写法并跑 `cargo test --release` 中的
`damage_recalc_body_from_tail` 回归。

## 场景格式 (wire JSON)

`.sim.ts` 用 camelCase, runner 映射后得到下面的 wire JSON (`scenarios/*.json` 与
`--scenario` 直跑用的就是这个格式, 字段一一对应):

```jsonc
{
  "name": "boot-from-scratch",
  "seed": 42,                 // 确定性随机 (地形/Math.random); seed 为 0 表示不播种
  "ticks": 20000,
  "cpu": {
    "limit": 100,             // 每 tick CPU 上限 (ms)
    "bucket": 10000,          // 初始 bucket
    "tick_limit": 500,        // 覆盖 tickLimit; 默认按官方公式 min(500, max(0, bucket) + limit) 逐 tick 计算
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
    "mineral": { "pos": [38, 38], "mineral_type": "H", "amount": 30000, "density": 2 },
    "structures": [
      { "type": "spawn", "pos": [25, 25], "energy": 300 },
      // store: 预置仓储 (中后期场景直接起步); level: 工厂等级
      { "type": "terminal", "pos": [20, 20], "store": { "energy": 50000, "H": 3000 } }
    ],
    // body: 字符串或带强化的 {"type": "work", "boost": "UO"} (boost 须与部件匹配, 见官方 BOOSTS)
    "creeps": [{ "name": "W1", "body": ["work", {"type": "carry", "boost": "KH"}, "move"], "pos": [26, 25] }]
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
部件名与部件数上限（50）、强化与部件匹配、`mineral.density` 1..4、结构类型、`terrain.raw` 长度 2500、`ticks > 0`、
`cpu.limit > 0`、`globalResetEvery > 0`。

断言类型：`rclAtLeast` / `rclReachedBy` / `creepCountAtLeast` / `cpuAvgBelow` /
`cpuP95Below` / `noErrors`。

`sources` 字段：`{"count": 2, "energy": 3000}` 在中线两侧的候选区内就近搜索非墙格放置
（不是严格对称），或 `{"positions": [[x,y],...]}` 显式指定。

### 机制回归测试

```bash
cargo test --release   # 推荐: 同时覆盖 release 优化后的代码生成 (见上方 rustc 已知问题)
```

| 测试文件 | 覆盖 |
|---|---|
| `tests/engine_rules.rs` | 用受控意图直接驱动引擎（不经 V8）：掉落衰减、Source 再生时序、疲劳步频、出口格越房与弹回、RCL 升级/降级数值、能量转移容量、容器承接掉落、部分拾取、意图顺序与互斥、`off` 停用、GCL 公式、事件日志；以及多房间移动隔离、交换对象优先、可见性判定、墙上路造价 ×150、容器首次衰减 +100、rampart 衰减与 hitsMax、建造障碍、拆除工地掉落、`renewCreep`/`recycleCreep`、`tower.repair`、`link.transferEnergy`；工业：结构激活取近、掉落资源分堆、采矿与 extractor 冷却、矿物再生、各类强化数值与 TOUGH 减伤、lab 反应/逆反应/boost/unboost、终端费用/截断/冷却、observer 可见性、processPower 与 GPL、工厂配方、工业字段快照对拍 |
| `tests/shim_apis.rs` | 微型 bundle 驱动 V8：PathFinder 跨房、地图 API、`moveTo` 端到端闭环（含 Memory 往返与 `_move` 缓存）；工业 API（`industry_apis_via_js`）的返回码与下一 tick 结果 |
| `tests/cpu_watchdog.rs` | CPU 看门狗中断与 isolate 恢复、超时 tick 的丢弃语义、未超时不误报、意图计价 0.2 CPU/组合 |
| `tests/memory_segments.rs` | `RawMemory` 段跨 tick 持久、需每 tick 重新激活、ID/数量/类型/长度校验、`RawMemory.set` 语义 |
| `tests/framework.rs` | 全局重置（全局变量丢失、Memory 与段保留）、场景加载校验的正/反用例、同场景同 seed 重放一致性（确定性） |
| `tests/cli_cpu_bucket.rs` | CLI 级集成：bucket 耗尽跳过（官方 "CPU bucket is empty"）与 `noErrors` 失败退出码、正常场景退出码 0、tickLimit 公式与场景覆盖值 |

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
  main.rs        CLI (含 stdin 场景 / stdout JSON 报告的管道协议)
bin/             screeps-sim 命令入口 (test 子命令 → runner, 其余透传引擎)
runner/          .sim.ts 测试 runner (发现/加载/运行/汇总, 零依赖)
index.mjs        编程 API (defineSim / runSim)
index.d.ts       .sim.ts 用例的完整类型定义
examples/        示例用例 (smoke 自包含可跑) + fixtures/min-bot.js 陪跑 bundle
scenarios/       wire JSON 场景 (--scenario 直跑用)
tests/           回归测试 (engine_rules / shim_apis / cpu_watchdog / memory_segments / framework)
```
