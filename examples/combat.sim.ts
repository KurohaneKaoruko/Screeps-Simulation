// 战斗演示: 激活安全模式 (拦截敌方攻击) + 发射核弹 (在飞核弹进入 FIND_NUKES)。
// 断言只用通用项 (noErrors) —— 数值语义由 tests/combat.rs 在引擎层逐条锁定。
// 看 bot 视角的战斗状态: screeps-sim test examples/combat.sim.ts --verbose
import { defineSim } from 'screeps-sim'

export default defineSim({
	name: 'combat',
	description: 'controller.activateSafeMode + nuker.launchNuke + FIND_NUKES',
	seed: 1,
	ticks: 120,
	bundle: './fixtures/combat-bot.js',
	rooms: [
		{
			name: 'W5N8',
			terrain: { kind: 'flat' },
			// RCL8 才有 nuker; safeModeAvailable 预置 1 次 (短场景免去先升级控制器)
			controller: { level: 8, pos: [25, 25], safeModeAvailable: 1 },
			structures: [
				// 满仓 nuker: 发射消耗 300000 energy + 5000 G
				{ type: 'nuker', pos: [40, 40], store: { energy: 300000, G: 5000 } },
			],
			creeps: [{ name: 'defender', body: ['attack', 'move'], pos: [10, 10] }],
		},
	],
	asserts: [{ kind: 'noErrors' }],
})
