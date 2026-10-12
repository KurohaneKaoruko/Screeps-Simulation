# 深度分析:迈向高性能、高保真的 Screeps 模拟器

> 基于代码热路径审查 + 受控基准测量 (2026-10, 本文档随实现推进更新)

## ✅ P1 批次已交付 (2026-10-12, 接 P0)

- **直接序列化器**: `build_snapshot` 直写 String, 不建 `serde_json::Value` 树;
  `build_snapshot_value` 保留为对拍基准 (parity 测试保证两者语义一致, 富对象世界 + 转义字符)。
- **terrain 不再重发**: 地形不可变 + shim `terrainCache` 常驻 → 仅 isolate 新建/重置后的
  首个快照携带 (`include_terrain`), 9 房场景每 tick 省 ~20KB 序列化。
- **意图录制/回放**: `--record-intents x.jsonl` / `--replay x.jsonl` (纯引擎, 零 V8);
  CLI 测试验证回放的世界状态哈希与原运行逐位一致。
- **runner 并行**: `test -j N` (每用例独立引擎进程, 原序渲染; cpu*Below 断言警告)。
- 实测 (3000 tick, min-bot): 1 房 8815→**21179 t/s**, 9 房 5806→**17064 t/s**
  (对比最初基线 6560/4689 = **3.2×/3.6×**); 快照相位 0.051→0.004ms;
  回放模式 **113,868 t/s**。剩余底噪在 JS 侧 (JSON.parse + buildGame 重建),
  下一步候选: V8 startup snapshot (isolate 重建 17ms) 与 shim 重建的增量对象包装 (语义风险需对拍)。

## ✅ P0 批次已交付 (2026-10-12)

- 快照边界 3 次拷贝 → 1 次 (op take 转移所有权); 意图解析 3 次 → 1 次
  (整 tick 结果单次反序列化为强类型 `TickOutcome`); terrain 每 tick 重序列化 → 构建期缓存。
- 实测: 1 房 6560→8815 t/s (**+34%**), 9 房 4689→5806 t/s (**+24%**)。
- 分相计时进报告 (`profile.snapshot/js/engine`, JSON + 人类报告 + CSV);
  JSON 报告新增 `stateHash` (FNV-1a 64, 手工实现跨版本稳定)。
- 黄金哈希回归: `world_state_hash_golden` (受控意图流 12 tick → 基线
  `0x886da1f4087983e9`, 双跑一致性), 引擎任何数值/顺序漂移立刻爆红。
- 下一步瓶颈 (相位数据证实): 快照的 `serde_json::Value` 树构建+序列化占 1 房 tick 的 ~46%;
  9 房快照比 1 房贵 0.028ms (不可见邻房 terrain 仍整体重发) → 指向 P1-1 增量快照与直接序列化器。

## 0. 现状画像 (实测)

受控基准 (3000 tick, `--json-report -`, 同机):

| 配置 | 吞吐 | 每 tick 成本 | 说明 |
|---|---|---|---|
| 1 房 + 空 bot | 6560 t/s | 0.15 ms | **模拟器底噪**: 快照序列化 + V8 往返 + shim 重建 + 引擎结算 |
| 9 房 + 空 bot | 4689 t/s | 0.21 ms | +8 个不可见邻房也要序列化 terrain (9×2500 字符/tick) |
| 9 房 + 真实 bot | 535 t/s | 1.87 ms | bot 自身 wall-clock ~1.5ms (cpu avg 2.12ms 含意图计价) |
| 同上 + 每 100t 全局重置 | — | +17 ms/次 | isolate 重建 = lodash(540KB)+shim+bundle 全量重编译 |

结论:
- 真实 bot 长跑时模拟器开销占比 ~10-30%, 底噪决定吞吐**上限**;
- 轻负载场景 (早期开局、机制单测) 底噪占比 >90%, 是主要优化对象;
- 底噪的构成 (按嫌疑排序): 快照三重拷贝 + `serde_json::Value` 树 → `terrain` 每 tick 重序列化
  → `JSON.parse`/`buildGame` 重建 → 意图/内存在 Rust/JS 边界重复解析。

### 每 tick 数据流解剖 (含冗余标记)

```
Rust 引擎                         V8 / JS shim
─────────                         ────────────
build_snapshot → Value 树          [alloc: 整棵 JSON 树]
  (snapshot.rs:21)                ├─ terrain: 所有房间×2500 字符, 无条件, 无缓存 ✗
Value.to_string()  (main.rs)      [copy #1]
TICK_IO.snapshot = ..to_string()  [copy #2 全量拷贝] (runtime.rs:229)
                                  JSON.parse(snapshot)        [parse #1] (screeps.js:2657)
                                  buildGame() 重建 Game/类实例 [alloc: 每 tick 全量]
Memory: 同样模式 in(2 copy+parse) / out(stringify+parse)
loop() 运行 bot
op_sim_result(result: String)     [copy] (runtime.rs:53)
from_str::<Value>(raw)            [parse #1] (runtime.rs:272)
value["intents"].to_string()      [copy + parse #2] (runtime.rs:276)
IntentList::parse(intents_json)   [parse #3!] (main.rs:294)
```

---

## A. 性能路线图 (按 ROI 排序)

### P0-1 消除边界冗余拷贝/解析 —— 预期底噪 2-3×

| 问题 | 位置 | 方案 |
|---|---|---|
| 快照 3 次全量拷贝 | main.rs / runtime.rs:229,43 | 单次序列化直写; `TICK_IO` 存 `&str` 不可行则 `Rc<str>`/swap; op 返回 `#[string]` 时只做必要的一次 V8 字符串创建 |
| `serde_json::Value` 树中间层 | snapshot.rs | 手写序列化器直接产 JSON 字符串 (对象字段固定, 用 `itoa`/`ryu` 或直接 `write!`), 消除整棵 Value 树分配 |
| intents 解析 3 次 | runtime.rs:272-276, main.rs:294 | 结果结构体一次性 `from_str` 成强类型 (`IntentsResult { intents: IntentList, memory: String, .. }`), 意图只解析 1 次 |
| memory 双重 parse | runtime.rs | 同上并入结果结构体 |
| terrain 每 tick 重序列化 | snapshot.rs:22-27 | `Room` 缓存 `terrain_json: String` (tick 期间地形不可变); 只对**可见房**提供 terrain, 其余按需 (核对 shim 的 `Game.map` 用法) |

### P0-2 `--profile` 相位计时 —— 让优化数据驱动

报告与 CSV 增加: `snapshot_us / js_us / result_parse_us / engine_us / memory_us`,
`--profile` 打印 P50/P95 分相直方图。没有这个, 后续所有优化都是盲飞。

### P1-1 房间级增量快照 —— 大世界 5-10×

多数 tick 每房只有 `progress/energyAvailable/hits/fatigue/ttl` 等少数字段变化:
- `Room` 持有 `objects_json: HashMap<Id, CachedJSON>` + 脏标记 (tick 结算时标记变动对象);
- 快照时拼接缓存片段, 只重序列化脏对象; 房间整体不可见→可见时全量重建;
- `events` 本来就只含当 tick。
风险: 缓存失效遗漏 → 语义错误, 必须配「快照哈希对拍测试」(增量 vs 全量逐 tick 比对, CI 强制)。

### P1-2 V8 startup snapshot —— isolate 重建 17ms → ~2ms

deno_core 支持 `create_snapshot`: 把 lodash + shim(+ 空 bootstrap) 烘焙进 snapshot blob,
isolate 创建从「编译 540KB 源码」变为「反序列化字节码」。收益:
- `globalResetEvery` 场景重建成本 -90%;
- runner 每用例冷启动 (每个用例独立进程) 同样受益;
- watch 模式/时间旅行回放的重放吞吐。

### P1-3 意图录制回放 (`--replay`) —— 真正"绕过 V8"的形态

跑场景时每 tick 意图落盘 `intents.jsonl`; 回归时纯 Rust 引擎回放, **零 JS** (引擎可不经 V8
直接驱动, tests/engine_rules.rs 已是此形态):
- 结算回归的极速形态 (预计 10 万+ t/s), 配世界状态哈希 golden test;
- bot A/B 对比: 两版本各录意图流, diff 到第一个分歧 tick —— 精确定位行为变化起点;
- 固定意图流下调引擎参数, 研究世界长跑演化。
(`--record-intents intents.jsonl` + `--replay intents.jsonl [--no-bundle]`)

### P1-4 runner 多场景并行 (`--jobs N`)

每用例独立进程天然可并行; 约定: 含 `cpuAvgBelow/cpuP95Below` 断言的套件建议串行
(并行 CPU 争抢会污染墙钟口径), 文档标注 + 默认串行。

### P2 引擎内部数据结构

- 对象表 `BTreeMap<Id, Obj>` → `SlotMap`/`IndexMap` (意图结算热路径查找);
- 意图结算与移动仲裁的临时 Vec 复用 (arena);
- `objects.values()` 迭代顺序确定性已有 (BTreeMap) —— 换结构时保持序确定性 (创建序)。

### P3 其他

- console 行缓冲已有; CSV 写入改 bufwriter;
- `summarize()` 每 tick 全对象扫描 → 增量维护;
- 编译: `codegen-units=1`, `lto=fat` 试试; rustc 已知 miscompile 项保持 clamp 写法。

---

## B. 保真度路线图 ("完美"的另一半是正确)

### 经济闭环缺口 (经济 bot 的硬缺口, 优先)

1. **boost / 实验室**: 反应配方、`lab.runReaction`、boost 对部件数值的倍率
   (tough +100% hits / attack ×2 / heal ×2 / dismantle ×2 / move 肥劳减半 …);
   没有 boost 的战斗/搬运数值都不完整。
2. **market**: `Game.market` 订单/成交/历史、terminal 跨房交易 (含 NPC 订单可选)。
3. **factory/commodities**: 产业链 (对高级经济 bot)。
4. 矿物采集 (extractor + mineral 再生 5w tick)。

### PvP/PvE 缺口

5. **safeMode** 拦截攻击、**nuker** (landing→impact 两阶段)、**power creeps** (`PWR_*` 效果);
6. **NPC**: invader 定期袭击 (对外房 harvest 的惩罚, 官方平衡性核心)、source keeper、power bank;
7. ruin、construction site 过期、portal/highway。

### 终极正确性工具: 官方引擎对拍 (parity harness)

本地跑官方私服 (`screeps` npm / docker `screeps-docker`), 用 `screeps-server-mockup`
或直接 driver API 注入同一意图序列, 逐 tick 对比世界状态哈希:
- 官方侧写一个最小 bot (把收到的意图回放);
- 哈希函数两边一致 (规范化 JSON: 对象排序、数值格式);
- 分歧即 bug 清单, 自动生成 regression 场景。
这是"完美模拟器"的定义性工具 —— 比任何人肉对照 README 的近似表都强。

### 确定性回归基建

8. 世界状态哈希 (`--state-hash-every N` 进 CSV) + 新断言类型 `worldHashAtTick`:
   golden replay, 任何结算顺序/数值变化立刻爆红;
9. 能量守恒 proptest: 每 tick `采集+存量变化 == 消耗+衰减+掉落` 恒等式。

### 细节对齐清单 (小但真实的偏差)

10. PathFinder 跨房计价 (出口+入口两步→官方同节点);
11. 可见性残留: 官方在最后一个 creep 离开后房间仍可见 1 tick (`visibility decay`);
12. 事件类型补全 (对照官方 `EVENT_*` 全表: RENEW_CREEP / SPAWN 等);
13. `Game.map` room status (novice/reserved)、`describeExits` 完整性;
14. ID 顺序 = 创建顺序对 `Game.creeps` 迭代序的影响 (与官方 `_id` 时间戳基一致);
15. 地形生成: 支持从真实服导出的 raw 喂入已有; `generated` 保持近似但文档标注。

---

## C. 开发体验路线图 (产品化)

1. **`--inspect` CDP 调试**: deno_core inspector 会话, Chrome DevTools 断点进 bot 代码,
   配合 `--ticks N --break-at T` —— bot 开发回路的质变;
2. **时间旅行**: 每 N tick 落盘 World 快照 (World 已可序列化), `screeps-sim replay --at T`
   恢复世界并 dump/续跑/单步 —— 定位"哪个 tick 开始坏的";
3. **A/B 对比**: 同场景两个 bundle, 报告 diff (RCL 时间线/CPU 曲线/能量流);
4. **watch 模式**: bundle 变更 → 自动重建重跑最近场景;
5. **可视化**: 逐 tick 房间 ASCII/HTML 渲染 (已有 terrain.render 雏形), Prometheus 指标导出;
6. **发布工程**: GitHub Releases 预编译二进制 + npm `optionalDependencies` 按平台分发
   (esbuild 模式, 消费者免 Rust 工具链), CI 跑 `cargo test + npm test + criterion 预算`。

---

## D. 工程化

- crate 拆分: `screeps-engine` (纯 Rust, 无 V8) / `screeps-sim-runtime` (V8 宿主) / CLI / npm;
  engine lib 化后可被对拍工具、渲染器、未来 GUI 复用;
- criterion 基准: snapshot 构建 / tick 结算 / movement, CI 卡性能预算;
- shim 未实现桩清单自动进报告 warnings (bot 侧可知模拟覆盖度)。

---

## 优先级矩阵

> 已交付: ✅ `.sim.ts` 测试 runner + npm 包 (`screeps-sim test/check`, 见 README) ·
> ✅ 模拟前静态检查 (场景语义 + bot bundle 常量拼写/参数顺序/node 泄漏/覆盖度, `runner/check.mjs` + `runner/bundle-check.mjs`) ·
> ✅ stdin/stdout 管道协议 (`--scenario -` / `--json-report -`, 报告内嵌断言结果)

| 项 | 影响 | 成本 | 建议批次 |
|---|---|---|---|
| P0-1 边界拷贝/解析消除 | 高 (底噪 2-3×) | 低-中 | **第一批** |
| P0-2 相位计时 | 高 (数据驱动前提) | 低 | **第一批** |
| 快照哈希 golden test | 高 (安全网) | 低 | **第一批** |
| P1-1 增量快照 | 高 (大世界 5-10×) | 中-高 | 第二批 |
| P1-2 V8 startup snapshot | 中-高 | 中 | 第二批 |
| P1-3 意图录制回放 | 高 (回归吞吐 ×100) | 低-中 | 第二批 |
| P1-4 runner 并行 | 中 (套件吞吐) | 低 | 第二批 |
| boost/lab + market | 高 (经济保真) | 高 | 第三批 |
| 官方对拍 harness | 极高 (正确性闭环) | 高 | 第三批 |
| CDP 调试 / 时间旅行 | 高 (DX 质变) | 中-高 | 第四批 |
| NPC / nuke / power | 中 | 中 | 第四批 |
| 预编译二进制分发 | 中 (普及) | 中 | 第四批 |
