// smoke: 最小自测用例 —— 不依赖真实 bot, 验证 .sim.ts 管道本身 (npm test 跑的就是它)。
// 用例级 bundle 指向仓库内 fixture, 相对路径基于本文件。
import { defineSim } from 'screeps-sim'

export default defineSim({
	name: 'smoke',
	description: '空转 bot + 1 个初始单位, 验证 runner 管道与基础断言',
	seed: 7,
	ticks: 50,
	bundle: './fixtures/min-bot.js',
	rooms: [
		{
			name: 'W1N1',
			terrain: { kind: 'flat' },
			sources: { count: 1, energy: 3000 },
			controller: { level: 1, pos: [28, 27] },
			structures: [{ type: 'spawn', pos: [25, 25], energy: 300 }],
			creeps: [{ name: 'idle1', body: ['move'], pos: [26, 25] }],
		},
	],
	asserts: [
		{ kind: 'noErrors' },
		{ kind: 'creepCountAtLeast', value: 1 },
	],
})

// 同文件可以再导出更多用例 (runner 会逐一执行); skip: true 的用例只列出、不运行:
export const smokeFailing = defineSim({
	name: 'smoke-failing',
	description: '故意失败的用例, 演示失败输出 (去掉 skip 即可看到 ✗ 效果)',
	skip: true,
	seed: 7,
	ticks: 20,
	bundle: './fixtures/min-bot.js',
	rooms: [
		{
			name: 'W1N1',
			terrain: { kind: 'flat' },
			controller: { level: 1, pos: [28, 27] },
			structures: [{ type: 'spawn', pos: [25, 25], energy: 300 }],
		},
	],
	asserts: [{ kind: 'creepCountAtLeast', value: 99 }],
})
