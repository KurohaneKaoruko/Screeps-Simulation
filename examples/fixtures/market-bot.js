// 市场演示 bot: 与外部卖单成交 + 挂自己的卖单 + 读取 credits / orders / history。
// 配合 examples/market.sim.ts 使用: screeps-sim test examples/market.sim.ts --verbose
exports.loop = function () {
	const room = Game.rooms['W5N8'];
	if (!room) return;
	const market = Game.market;

	if (Game.time === 1) {
		// 与外部卖单成交: 从 W7N8 买入 100 H (运输能量费由我们的终端出)
		const sell = market.getAllOrders({ resourceType: RESOURCE_HYDROGEN }).find((o) => o.type === ORDER_SELL);
		if (sell) market.deal(sell.id, 100, 'W5N8');
	}
	if (Game.time === 2) {
		// 挂一张自己的卖单: 手续费 = 3 * 50 * 5% = 7.5 Cr (下单即扣)
		market.createOrder({ type: ORDER_SELL, resourceType: RESOURCE_HYDROGEN, roomName: 'W5N8', price: 3, totalAmount: 50 });
	}

	const state = {
		t: Game.time,
		credits: market.credits,
		allOrders: market.getAllOrders().length,
		myOrders: Object.keys(market.orders).length,
		hHistory: market.getHistory(RESOURCE_HYDROGEN).length,
	};
	Memory.market = state;
	if (Game.time % 50 === 0) console.log(JSON.stringify(state));
};
