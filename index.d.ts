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
	/** 初始可用安全模式次数 (官方 safeModeAvailable), 默认 0 */
	safeModeAvailable?: number
}

export interface MineralSpec {
	pos: [number, number]
	/** 矿物类型, 默认 'H' */
	type?: string
	amount?: number
}

export interface StructureSpec {
	type: string // spawn | extension | road | wall | rampart | link | storage | tower | observer | powerSpawn | extractor | lab | terminal | container | nuker | factory
	pos: [number, number]
	energy?: number
	hits?: number
	/** 敌方结构 (FIND_HOSTILE_STRUCTURES 目标) */
	enemy?: boolean
}

export interface CreepSpec {
	name: string
	body: BodyPart[]
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
