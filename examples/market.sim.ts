// 市场演示: 与外部卖单成交 (买入) + 挂自己的卖单 (收 5% 手续费)。
// 断言只用通用项 (noErrors) —— 市场数值语义由 tests/market.rs 在引擎层逐条锁定。
// 看 bot 视角的市场状态: screeps-sim test examples/market.sim.ts --verbose
import { defineSim } from 'screeps-sim'

export default defineSim({
	name: 'market',
	description: '外部卖单成交 (买入 H) + 自挂卖单 (5% 手续费) + credits/orders/history',
	seed: 1,
	ticks: 120,
	bundle: './fixtures/market-bot.js',
	market: {
		credits: 10000,
		// 外部卖单: W7N8 卖 1000 H, 单价 2 Cr (距 W5N8 两房, 运输能量费由我们接单方出)
		orders: [{ type: 'sell', resourceType: 'H', roomName: 'W7N8', price: 2, totalAmount: 1000 }],
	},
	rooms: [
		{
			name: 'W5N8',
			terrain: { kind: 'flat' },
			controller: { level: 6, pos: [25, 25] },
			// 终端是市场交易的落点: 需要能量 (付运输费) 与容量 (接货)
			structures: [{ type: 'terminal', pos: [20, 20], store: { energy: 100000 } }],
		},
	],
	asserts: [{ kind: 'noErrors' }],
})
