// runner 自测 (npm test 第一环): 不启动引擎, 直接验证 check 规则。
// 覆盖: 场景静态检查 (check.mjs) / bundle 静态检查 (bundle-check.mjs) / 常量表同步。
import { checkScenario } from './check.mjs'
import { checkBundle } from './bundle-check.mjs'
import { SCREEPS_CONSTANTS } from './screeps-constants.mjs'
import { extractShimConstants } from './gen-constants.mjs'
import { readFileSync } from 'node:fs'

let failed = 0
function expect(cond, label) {
	if (cond) {
		console.log(`  ✓ ${label}`)
	} else {
		failed++
		console.error(`  ✗ ${label}`)
	}
}

console.log('场景静态检查 (check.mjs)')
{
	const good = {
		name: 'ok',
		ticks: 100,
		rooms: [
			{
				name: 'W1N1',
				controller: { level: 2, pos: [28, 27] },
				sources: { count: 2 },
				structures: [
					{ type: 'spawn', pos: [25, 25], energy: 300 },
					{ type: 'extension', pos: [26, 25] },
				],
				creeps: [{ name: 'w', body: ['work', 'move'], pos: [24, 25] }],
			},
		],
		asserts: [{ kind: 'noErrors' }],
	}
	let r = checkScenario(good)
	expect(r.errors.length === 0, `合法场景无错误 (实际: ${r.errors.map((e) => e.message).join('; ')})`)

	const bad = {
		name: 'bad',
		ticks: 100,
		rooms: [
			{ name: 'W1N1', controller: { level: 9, pos: [50, 0] }, sources: { positions: [] } },
			{ name: 'W1N1' },
			{
				name: 'X1Y1',
				structures: [{ type: 'ufo', pos: [1, 1] }],
				creeps: [
					{ name: 'a', body: ['wing'], pos: [2, 2] },
					{ name: 'a', body: ['move'], pos: [3, 3] },
				],
			},
			{
				name: 'W1N2',
				creeps: [{ name: 'a', body: ['move'], pos: [1, 1] }], // 与 W1N1 的 a 全局重名
			},
		],
		asserts: [
			{ kind: 'rclReachedBy', value: 9, tick: 500 },
			{ kind: 'unknownKind' },
		],
	}
	r = checkScenario(bad)
	const msgs = r.errors.map((e) => e.message).join('\n')
	expect(r.errors.length >= 8, `非法场景应聚合报错 (得到 ${r.errors.length} 个):\n${msgs}`)
	expect(msgs.includes('非法房名'), '房名校验')
	expect(msgs.includes('重复定义'), '房间重名')
	expect(msgs.includes('超出') || msgs.includes('0..8'), 'controller 等级')
	expect(msgs.includes('坐标') || msgs.includes('0..49'), '坐标越界')
	expect(msgs.includes('未知部件'), '部件校验')
	expect(msgs.includes('未知结构类型'), '结构类型')
	expect(msgs.includes('全游戏唯一'), '全局单位重名')
	expect(msgs.includes('未知断言类型'), '断言类型')

	// 不可能通过的断言: tick > 场景 ticks
	r = checkScenario({
		name: 'imp',
		ticks: 100,
		rooms: [{ name: 'W1N1' }],
		asserts: [{ kind: 'rclReachedBy', value: 2, tick: 500 }],
	})
	expect(r.errors.some((e) => e.message.includes('不可能')), '断言 tick 超出场景 ticks → 错误')

	// RCL 超限结构 → 警告 (off 行为)
	r = checkScenario({
		name: 'offl',
		rooms: [
			{
				name: 'W1N1',
				controller: { level: 1 },
				structures: Array.from({ length: 10 }, (_, i) => ({ type: 'extension', pos: [10 + i, 10] })),
			},
		],
	})
	expect(r.warnings.some((w) => w.message.includes('off')), 'RCL 超限结构 → off 警告')

	// 孤立房间 → 警告
	r = checkScenario({
		name: 'iso',
		rooms: [
			{ name: 'W1N1', structures: [{ type: 'spawn', pos: [25, 25] }] },
			{ name: 'W1N3' }, // 与 W1N1 不相邻
		],
	})
	expect(r.warnings.some((w) => w.message.includes('不连通')), '孤立房间 → 连通性警告')

	// NPC: keeperLair / powerBank 是合法结构, 且不吃 RCL 上限警告
	r = checkScenario({
		name: 'npc-ok',
		npcs: { invaders: { every: 8000, first: 3000, lifetime: 15000, squad: 3 } },
		rooms: [
			{
				name: 'W1N1',
				sources: { positions: [[18, 22]] },
				structures: [
					{ type: 'keeperLair', pos: [20, 22] },
					{ type: 'powerBank', pos: [10, 10], store: { power: 3000 } },
				],
			},
		],
	})
	expect(r.errors.length === 0, `NPC 场景合法 (实际: ${r.errors.map((e) => e.message).join('; ')})`)
	expect(!r.warnings.some((w) => w.message.includes('上限')), 'keeperLair/powerBank 无 RCL 上限警告')

	// NPC: invader 参数非法 → 报错; lair 无 source → 警告
	r = checkScenario({
		name: 'npc-bad',
		npcs: { invaders: { every: 0, squad: 0, lifetime: -1 } },
		rooms: [{ name: 'W1N1', structures: [{ type: 'keeperLair', pos: [20, 22] }] }],
	})
	const npcPaths = r.errors.map((e) => e.path).join('\n')
	expect(
		npcPaths.includes('npcs.invaders.every') &&
			npcPaths.includes('npcs.invaders.squad') &&
			npcPaths.includes('npcs.invaders.lifetime'),
		'npcs.invaders 参数校验'
	)
	expect(r.warnings.some((w) => w.message.includes('没有 source')), 'keeperLair 无 source → 警告')

	// market: 合法订单场景通过
	r = checkScenario({
		name: 'market-ok',
		market: {
			credits: 10000,
			orders: [{ type: 'sell', resourceType: 'energy', roomName: 'W1N1', price: 0.5, totalAmount: 1000 }],
		},
		rooms: [{ name: 'W1N1' }],
	})
	expect(r.errors.length === 0, `market 场景合法 (实际: ${r.errors.map((e) => e.message).join('; ')})`)

	// market: 非法 credits / 订单参数逐项报错
	r = checkScenario({
		name: 'market-bad',
		market: { credits: -1, orders: [{ type: 'hold', resourceType: 'H', roomName: 'NOPE', price: 0, totalAmount: 0 }] },
		rooms: [{ name: 'W1N1' }],
	})
	const marketPaths = r.errors.map((e) => e.path).join('\n')
	expect(
		marketPaths.includes('market.credits') &&
			marketPaths.includes('market.orders[0].type') &&
			marketPaths.includes('market.orders[0].roomName') &&
			marketPaths.includes('market.orders[0].price') &&
			marketPaths.includes('market.orders[0].totalAmount'),
		'market 订单参数校验'
	)
}

console.log('bundle 静态检查 (bundle-check.mjs)')
{
	let r = checkBundle('exports.loop = function () { const s = Game.spawns.Spawn1; if (creep.transfer(t, RESOURCE_ENERGY) === OK) {} };\n')
	expect(r.errors.length === 0, `正常代码无错误 (实际: ${r.errors.map((e) => e.message)})`)

	r = checkBundle(`exports.loop = function () {
	if (creep.transfer(RESOURCE_ENERGY, target) === ERR_NOT_IN_RANGE) {}
};`)
	expect(r.errors.some((e) => e.message.includes('target 在前')), 'transfer 参数顺序颠倒')

	r = checkBundle(`exports.loop = function () { spawn.spawnCreep('harv', [WORK, MOVE]); };`)
	expect(r.errors.some((e) => e.message.includes('body 数组在前')), 'spawnCreep 参数顺序颠倒')

	r = checkBundle(`exports.loop = function () { if (ret === ERR_NOT_IN_RANG) creep.move(1); };`)
	expect(r.errors.some((e) => e.message.includes('ERR_NOT_IN_RANG')), '常量拼写错误 (ERR_NOT_IN_RANG)')

	r = checkBundle(`exports.loop = function () { if (process.env.NODE_ENV) console.log(1); require('lodash'); };`)
	expect(r.errors.some((e) => e.message.includes('node 环境')), 'node 泄漏 (process/require)')

	r = checkBundle(`exports.loop = function () { setTimeout(() => {}, 100); };`)
	expect(r.warnings.some((w) => w.message.includes('setTimeout')), 'setTimeout 警告')

	// 市场 API 已实现: 不再报覆盖度错误/警告
	r = checkBundle(`exports.loop = function () { Game.market.deal(id, 1); Game.market.createOrder({ type: ORDER_BUY, resourceType: RESOURCE_ENERGY, roomName: 'W1N1', price: 1, totalAmount: 100 }); x = Game.market.credits; y = Game.market.getAllOrders(); z = Game.market.getHistory(RESOURCE_ENERGY); };`)
	expect(r.errors.length === 0 && r.warnings.length === 0, `市场 API 不再被标为未实现 (实际: ${[...r.errors, ...r.warnings].map((e) => e.message)})`)

	// 工业 API 已实现: 不再报覆盖度错误/警告
	r = checkBundle(`exports.loop = function () { lab.runReaction(a, b); lab.boostCreep(c); lab.unboostCreep(c); terminal.send(RESOURCE_ENERGY, 100, 'W1N1'); factory.produce(RESOURCE_UTRIUM_BAR); ps.processPower(); obs.observeRoom('W1N1'); Game.market.calcTransactionCost(100, 'W1N1', 'W2N2'); x = Game.gpl.level; };`)
	expect(r.errors.length === 0 && r.warnings.length === 0, `工业 API 不再被标为未实现 (实际: ${[...r.errors, ...r.warnings].map((e) => e.message)})`)

	r = checkBundle(`exports.loop = function () { nuker.launchNuke(pos); };`)
	expect(r.errors.some((e) => e.message.includes('nuker')), 'nuker.launchNuke → error')

	r = checkBundle(`exports.loop = function () { WebAssembly.instantiate(bytes); };`)
	expect(r.warnings.some((w) => w.message.includes('异步')), 'WebAssembly.instantiate 异步警告')

	// bot 自己声明的同前缀常量 / 属性访问 / 对象键 / 字符串 不是官方常量引用
	r = checkBundle(`var CREEP_LIFE = 1500, POWER_BATCH = 2;
const { TOWER_URGENT } = cfg;
function LAB_HELPER() {}
exports.loop = function () { x = CREEP_LIFE + POWER_BATCH + TOWER_URGENT + o.SPAWN_Q + LAB_HELPER(); y = { ENERGY_Z: 1 }; z = 'BOOST_Q'; };`)
	expect(r.errors.length === 0, `bot 自定义常量不误报 (实际: ${r.errors.map((e) => e.message)})`)
	// 箭头函数体内的 `;` 会截断声明扫描: 第二个声明符 (跨行的全大写常量) 不应被误报为拼写错误
	r = checkBundle(`const numAt = () => {
	const a = 1
	return a
}, SPAWN_SLOT_OFFSETS = [10, 20]
exports.loop = function () { y = SPAWN_SLOT_OFFSETS.length; };`)
	expect(r.errors.length === 0, `箭头体后的多声明符常量不误报 (实际: ${r.errors.map((e) => e.message)})`)
	r = checkBundle(`var CREEP_LIFE = 1500;\nexports.loop = function () { w = c ? FIND_SOURCEZ : 0; };`)
	expect(r.errors.some((e) => e.message.includes('FIND_SOURCEZ')), '三元表达式里的拼写错误仍报')

	// 注释里的命中不应报错
	r = checkBundle('// TODO: process.env 优化\nexports.loop = function () {};')
	expect(r.errors.length === 0, '注释中的命中不误报')
}

console.log('常量表同步 (screeps-constants.mjs ↔ src/js/screeps.js)')
{
	const shimSrc = readFileSync(new URL('../src/js/screeps.js', import.meta.url), 'utf8')
	const extracted = new Set(extractShimConstants(shimSrc))
	expect(
		extracted.size === SCREEPS_CONSTANTS.size &&
			[...extracted].every((n) => SCREEPS_CONSTANTS.has(n)),
		`常量表同步 (${SCREEPS_CONSTANTS.size} 个; shim 变化后跑 node runner/gen-constants.mjs)`
	)
}

if (failed) {
	console.error(`\n${failed} 项自测失败`)
	process.exit(1)
}
console.log('\n自测全部通过')
