// boot-from-scratch: 空房间从零启动 (等价 scenarios/boot-from-scratch.json, camelCase 风格)。
// TS 的好处: 邻房用表达式生成, 不用复制 8 遍 JSON。
import { defineSim } from 'screeps-sim'

/** 主房周围的 8 个邻房 (跨房视野/远征行为的舞台) */
const neighbors = ['W4N7', 'W4N8', 'W4N9', 'W5N7', 'W5N9', 'W6N7', 'W6N8', 'W6N9'].map((name) => ({
	name,
	terrain: { kind: 'flat' } as const,
	sources: { count: 2, energy: 3000 },
}))

export default defineSim({
	name: 'boot-from-scratch',
	description: '空房间从零启动 (基线: RCL2 @ t650; 注意: 当前 bot 升级停摆会导致 RCL 在 ~t10650 降回 1)',
	seed: 42,
	ticks: 20000,
	cpu: { limit: 100, bucket: 10000 },
	deterministicRandom: true,
	alwaysVisible: false,
	rooms: [
		{
			name: 'W5N8',
			terrain: { kind: 'flat' },
			sources: { positions: [[18, 22], [32, 30]] },
			controller: { level: 1, pos: [28, 27] },
			mineral: { pos: [38, 38], type: 'H', amount: 30000 },
			structures: [{ type: 'spawn', pos: [25, 25], energy: 300 }],
			creeps: [],
		},
		...neighbors,
	],
	asserts: [
		{ kind: 'rclReachedBy', value: 2, tick: 5000 },
		{ kind: 'creepCountAtLeast', value: 2 },
		{ kind: 'noErrors' },
	],
})
