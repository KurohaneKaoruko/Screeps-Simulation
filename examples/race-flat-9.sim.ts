// race-flat-9: 9 个平坦双矿房间极限冲级场景 (等价 scenarios/race-flat-9.json)。
// 中央主房 W5N8 (spawn + 双 source) + 8 个平坦双矿邻居 (中立控制器, 可作外矿/储备扩张位),
// 用于验证 race 分支 (src/branch/race) 的最快 RCL 冲级曲线。
// 运行: screeps-sim test examples/race-flat-9.sim.ts --bundle <race 分支构建出的 dist/main.js>
import { defineSim } from 'screeps-sim'

const neighbors = ['W4N7', 'W4N8', 'W4N9', 'W5N7', 'W5N9', 'W6N7', 'W6N8', 'W6N9'].map((name, i) => ({
	name,
	terrain: { kind: 'flat' } as const,
	sources: { count: 2, energy: 3000 },
	// 中立控制器: 外矿房不预设归属, claim/reserve 语义与官服一致
	controller: { level: 0, mine: false },
	// 矿物类型错开, 避免 FIND_MINERALS 空集影响布局/情报逻辑
	mineral: { pos: [[40, 40], [40, 10], [10, 10], [25, 38], [25, 12], [10, 40], [40, 40], [10, 10]][i] as [number, number], type: ['H', 'Z', 'K', 'U', 'L', 'X', 'F', 'O'][i], amount: 30000 },
}))

export default defineSim({
	name: 'race-flat-9',
	description: '9 个平坦双矿房间极限冲级: 中央主房 + 8 邻居 (双 source 外矿), 40kt 停跑并输出各级 RCL 总耗时',
	seed: 7,
	ticks: 40000,
	cpu: { limit: 100, bucket: 10000 },
	deterministicRandom: true,
	// race 服 (私服冲级 mod) 的升级需求表: L5 需求减半, 其余同官服
	// (A/B 实测: 官服口径下本 bot L6 需 ~46kt, 与真实 race 服 40kt 内达 L6 的观察不符;
	//  L5 607.5k 时 L6@35100, 节奏与真实服务器一致)
	controllerLevels: [200, 45000, 135000, 405000, 607500, 3645000, 10935000, 0],
	rooms: [
		{
			name: 'W5N8',
			terrain: { kind: 'flat' },
			sources: { count: 2, energy: 3000 },
			// 官方 respawn 语义: RCL1 + 已激活 20000t 安全模式 + 1 次可用
			controller: { level: 1, pos: [28, 27], safeModeAvailable: 1, safeModeUntil: 20000 },
			mineral: { pos: [10, 40], type: 'O', amount: 30000 },
			structures: [{ type: 'spawn', pos: [25, 25], energy: 300 }],
			creeps: [],
		},
		...neighbors,
	],
	asserts: [
		{ kind: 'rclReachedBy', value: 6, tick: 38000 },
		{ kind: 'cpuP95Below', value: 25 },
		{ kind: 'noErrors' },
	],
})
