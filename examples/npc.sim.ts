// NPC 演示: source keeper (lair 生成) / power bank (衰减) / invader 周期侵袭。
// 断言只用通用项 (noErrors) —— NPC 数值语义由 tests/npc.rs 在引擎层逐条锁定。
// 看 bot 视角的 NPC 状态: screeps-sim test examples/npc.sim.ts --verbose
import { defineSim } from 'screeps-sim'

export default defineSim({
	name: 'npc',
	description: 'keeperLair 生成 keeper (tick 300) + powerBank 线性衰减 + invader 周期侵袭',
	seed: 1,
	ticks: 320,
	bundle: './fixtures/npc-bot.js',
	// 侵袭: 每 100 tick 一波, 首次 tick 50, 每波 2 个, 存活 40 tick
	npcs: { invaders: { every: 100, first: 50, lifetime: 40, squad: 2 } },
	rooms: [
		{
			name: 'W5N8',
			terrain: { kind: 'flat' },
			sources: { positions: [[18, 22]] },
			controller: { level: 1, pos: [28, 27] },
			structures: [
				{ type: 'spawn', pos: [25, 25], energy: 300 },
				// lair 绑定最近的 source, 每 300 tick 生成 2 个 keeper
				{ type: 'keeperLair', pos: [20, 22] },
				// powerBank: hits 2M, 内含 power, 每 100 tick 线性衰减 (5000 tick 归零)
				{ type: 'powerBank', pos: [10, 10], store: { power: 3000 } },
			],
		},
	],
	asserts: [{ kind: 'noErrors' }],
})
