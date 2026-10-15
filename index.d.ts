/**
 * screeps-sim — .sim.ts 用例类型定义
 *
 * 字段为 TS 友好的 camelCase; runner 负责映射到引擎的 wire JSON
 * (与 src/engine/scenario.rs 的 serde 结构一一对应)。
 */

export type BodyPart =
	| 'move'
	| 'work'
	| 'carry'
	| 'attack'
	| 'ranged_attack'
	| 'heal'
	| 'claim'
	| 'tough'

/** 带实验室强化的部件 (boost 须与部件匹配, 见官方 BOOSTS, 如 work+UO / carry+KH / tough+XGHO2) */
export interface BoostedBodyPart {
	type: BodyPart
	boost?: string
}

export type ResourceKey = string // 'energy' | 'K' | 'U' | 'L' | 'Z' | 'X' | 'G' | ...

/** 地形: 全平原 / 种子生成 / 50 行字符 / 2500 字符原始数字串 */
export type TerrainSpec =
	| { kind: 'flat' }
	| { kind: 'generated'; seed?: number }
	| { kind: 'rows'; rows: string[] } // 每行 50 字符: '.' 平原 '#' 墙 '~' 沼泽
	| { kind: 'raw'; data: string }

/** Source 放置: 自动 (数量/容量) 或显式坐标 */
export interface SourcesSpec {
	/** 自动放置数量 (1..3, 默认 2) */
	count?: number
	/** 每个 source 容量 (默认 3000) */
	energy?: number
	/** 显式坐标 (设置后忽略 count/energy) */
	positions?: [number, number][]
}

export interface ControllerSpec {
	/** 0..8 */
	level: number
	pos?: [number, number]
	progress?: number
	/** 默认 true; false 构造中立控制器 (claim/reserve 验证) */
	mine?: boolean
	/** 预置可用的安全模式激活次数 (官方 safeModeAvailable); 缺省 0 */
	safeModeAvailable?: number
}

export interface MineralSpec {
	pos: [number, number]
	/** 矿物类型, 默认 'H' */
	type?: string
	amount?: number
	/** 官方 density 1..4 (默认 2): 枯竭 50000t 后按 MINERAL_DENSITY[density] 再生 */
	density?: number
}

export interface StructureSpec {
	type: string // spawn | extension | road | wall | rampart | link | storage | tower | observer | powerSpawn | extractor | lab | terminal | container | nuker | factory | keeperLair | powerBank | portal | controller | invaderCore
	pos: [number, number]
	energy?: number
	hits?: number
	/** 敌方结构 (FIND_HOSTILE_STRUCTURES 目标) */
	enemy?: boolean
	/** 预置仓储, 如 terminal { energy: 50000, H: 3000 } / lab { energy: 2000, UO: 3000 } / powerBank { power: 3000 } (与 energy 同给时以此为准) */
	store?: Partial<Record<ResourceKey, number>>
	/** 工厂等级 (仅 factory) */
	level?: number
}

export interface CreepSpec {
	name: string
	/** 部件: 'work' 或带强化的 { type: 'work', boost: 'UO' } */
	body: (BodyPart | BoostedBodyPart)[]
	pos: [number, number]
	/** 剩余寿命 tick 数 (默认 1500 满寿命) */
	ttl?: number
	/** 初始携带资源, 如 { energy: 100 } */
	store?: Partial<Record<ResourceKey, number>>
	/** 敌方单位 (FIND_HOSTILE_CREEPS 目标) */
	enemy?: boolean
}

export interface RoomSpec {
	/** 形如 W5N8 / E12S3 */
	name: string
	terrain?: TerrainSpec
	sources?: SourcesSpec
	mineral?: MineralSpec
	controller?: ControllerSpec
	structures?: StructureSpec[]
	creeps?: CreepSpec[]
}

/** invader 侵袭波次配置 (缺省 = 无侵袭; 引擎常量 INVADER_DEFAULT_*) */
export interface InvaderSpec {
	/** 侵袭周期 tick (默认 12000) */
	every?: number
	/** 首次侵袭 tick (默认 6000) */
	first?: number
	/** invader 存活 tick 数, 到期撤离消失 (默认 15000) */
	lifetime?: number
	/** 每波数量 (默认 3) */
	squad?: number
}

/** NPC 配置: invader 侵袭; keeper / powerBank 由房间结构 keeperLair / powerBank 直接驱动 */
export interface NpcSpec {
	invaders?: InvaderSpec
}

/** 外部市场订单 (NPC / 他玩家): 供本玩家 deal, 无需真实终端或账户 */
export interface MarketOrderSpec {
	type: 'buy' | 'sell'
	resourceType: string
	/** 订单所在房间 (形如 W5N8); 成交时运输距离由此计算 */
	roomName: string
	/** 单价 (Cr/单位), 0.001 .. 1e8, 最多 3 位小数 */
	price: number
	totalAmount: number
}

/** 市场配置: 初始账户 credits + 外部订单 (下单收订单总价 5% 手续费; 运输能量费由接单方终端出) */
export interface MarketSpec {
	/** 初始账户 credits (默认 0) */
	credits?: number
	orders?: MarketOrderSpec[]
}

export type AssertSpec =
	| { kind: 'rclAtLeast'; value: number; room?: string }
	| { kind: 'rclReachedBy'; value: number; tick: number }
	| { kind: 'creepCountAtLeast'; value: number }
	| { kind: 'cpuAvgBelow'; value: number }
	| { kind: 'cpuP95Below'; value: number }
	| { kind: 'noErrors' }

export interface CpuSpec {
	/** 每 tick CPU 上限 (ms), 默认 100 */
	limit?: number
	/** 初始 bucket, 默认 10000 */
	bucket?: number
	/** 覆盖 tickLimit (默认按官方公式 min(500, max(0,bucket)+limit) 逐 tick 计算) */
	tickLimit?: number
	/** 超过 tickLimit 时强制中断当 tick (默认仅计量) */
	enforce?: boolean
	/** 确定性 CPU: Game.cpu.getUsed() 不读墙钟 (每次取时间推进 deterministicStep 毫秒 + 意图 0.2), Date.now 跟 Game.time 走;
	 *  同一场景每次运行逐位一致, bot 按 CPU 做决策时也能做 A/B 对比。CLI 等价开关 --deterministic-cpu */
	deterministic?: boolean
	/** 确定性模式下每次取时间推进的毫秒数, 默认 0.01 */
	deterministicStep?: number
}

export interface SimScenario {
	name: string
	description?: string
	/** 确定性随机种子 (地形/Math.random); 0 表示不播种 */
	seed?: number
	/** 默认运行 tick 数 (runner --ticks 覆盖) */
	ticks?: number
	cpu?: CpuSpec
	/** 初始 Memory JSON 字符串 */
	memory?: string
	/** 调试: 所有房间常可见 */
	alwaysVisible?: boolean
	/** 确定性 Math.random, 默认 true */
	deterministicRandom?: boolean
	/** >0 时每 N tick 重建 isolate 模拟全局重置 (全局变量丢失, Memory 保留) */
	globalResetEvery?: number
	/** NPC: invader 侵袭波次 (keeper/powerBank 由房间结构驱动) */
	npcs?: NpcSpec
	/** 市场: 初始 credits + 外部订单 (下单 5% 手续费 / 接单方付运输能量费) */
	market?: MarketSpec
	rooms: RoomSpec[]
	asserts?: AssertSpec[]
	/** 本用例专用 bundle 路径 (相对 .sim.ts 文件); 缺省: --bundle → ./dist/main.js */
	bundle?: string
	/** 跳过该用例 (仍会列出, 不运行) */
	skip?: boolean
}

/** defineSim 的返回值: 场景 + 用例标记 (仅作类型收紧, 不引入运行时行为) */
export interface SimCase extends SimScenario {
	readonly __simCase?: true
}

/** 引擎结构化报告 (对应 src/main.rs build_json_report) */
export interface SimReport {
	scenario: string
	ticks: number
	seconds: number
	ticksPerSecond: number
	cpu: {
		avg: number
		p50: number
		p95: number
		p99: number
		overLimitTicks: number
	}
	energy: { harvested: number; built: number; control: number; spawned: number }
	creeps: { produced: number; lost: number; alive: number }
	gcl: { level: number; progress: number; progressTotal: number }
	rclTimeline: Record<string, number>
	memoryPeakBytes: number
	totalErrors: number
	asserts: Array<{ kind: string; passed: boolean; detail: string }>
	fatalError: boolean
	final: {
		tick: number
		rcl: number
		progress: number
		creeps: number
		energyAvailable: number
		storage: number
	} | null
}

/** 在 .sim.ts 中声明一个模拟用例; 一个文件可 default 导出一个 + 具名导出多个 */
export function defineSim(scenario: SimScenario): SimCase

/** 编程直跑单用例, 返回结构化报告 */
export function runSim(
	sim: SimScenario,
	opts?: { bundle?: string; ticks?: number; verbose?: boolean }
): SimReport

/** 当前平台探测结果 (对应 runner/platform.mjs 的 TARGETS) */
export interface PlatformInfo {
	platform: NodeJS.Platform
	arch: string
	/** linux 才有: 'glibc' | 'musl' */
	libc: 'glibc' | 'musl' | null
	/** 平台包 key (如 linux-x64-gnu); 官方无预编译包时为 null */
	key: string | null
	/** 平台包名 (如 @screeps-sim/linux-x64-gnu) */
	packageName: string | null
	rustTarget: string | null
	supported: boolean
}

/** 引擎二进制的定位结果 (纯查询, 不启动引擎也不触发构建) */
export interface BinaryInfo {
	ok: boolean
	path: string | null
	/** 命中来源: 环境变量 / 源码构建 / 平台包 */
	source: string | null
	platform: PlatformInfo
	platformPackageInstalled: boolean
	/** 包内是否带 Rust 源码 (决定能否 cargo build) */
	sourceTree: boolean
}

/** 查引擎二进制与环境 (排障用; 等价 `screeps-sim doctor`, 但返回结构化数据) */
export function binaryInfo(): BinaryInfo

/** 二进制的候选路径 (按优先级: SCREEPS_SIM_BIN → target/release → 平台包) */
export function candidateBinaryPaths(): string[]

/** `screeps-sim doctor` 的结构化版本 */
export interface DoctorReport {
	package: { name: string; version: string; license: string | null }
	node: { version: string; required: string | null; ok: boolean | null }
	platform: PlatformInfo & { published: string[] }
	binary: {
		ok: boolean
		path: string | null
		source: string | null
		/** 引擎自报版本 (二进制过老时为 null) */
		version: string | null
		sizeBytes: number | null
		platformPackageInstalled: boolean
		sourceTree: boolean
	}
	project: {
		cwd: string
		config: boolean
		bundle: string | null
		bundleExists: boolean | null
		tsSupport: boolean
	}
	/** 二进制不可用时的排障步骤 */
	hint: string | null
}

export function doctorReport(cwd?: string): DoctorReport
