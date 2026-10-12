// 压力测试 bot: 把每 tick 的 CPU 预算烧到指定占比 (默认 ~99%, 可用 Memory.__stress.mul 放大)
// 负载刻意做成"真实形态"的混合, 而不是纯 Math.random 忙等 —— 这样压到的是 shim 的热路径:
//   ① PathFinder.search (重型寻路, 真实 bot 的 CPU 大头)
//   ② find() + lodash 迭代 (对象扫描)
//   ③ Memory 读写 (tick 末序列化成本)
//   ④ 校准忙等收尾 (sqrt 定点负载, 把 used 精确顶到目标线)
exports.loop = function () {
	const limit = Game.cpu.limit || 100
	const mul = (Memory.__stress && Memory.__stress.mul) || 1
	const target = Math.min(0.99 * mul * limit, 450) // 450 < 官方 tickLimit 上限 500

	let work = 0
	// ① 真实负载: 可见房间内重型寻路 (控制在目标的 70%, 留给其他负载)
	while (Game.cpu.getUsed() < target * 0.7) {
		for (const name of Object.keys(Game.rooms)) {
			const room = Game.rooms[name]
			const from = new RoomPosition(25, 25, name)
			const to = new RoomPosition(10 + (Game.time % 30), 10 + ((Game.time * 7) % 30), name)
			PathFinder.search(from, { pos: to, range: 3 }, { maxRooms: 1 })
			work++
		}
		// ② find + lodash 迭代
		const sources = _.filter(Game.rooms[Object.keys(Game.rooms)[0]].find(FIND_SOURCES), (s) => s.energy > 0)
		work += _.reduce(Game.creeps, (acc, c) => acc + c.pos.x + c.pos.y, sources.length)
	}
	// ③ Memory 负担 (环状历史, tick 末序列化)
	if (!Memory.__stress) Memory.__stress = {}
	if (!Memory.__stress.history) Memory.__stress.history = []
	Memory.__stress.history.push({ t: Game.time, w: work })
	if (Memory.__stress.history.length > 200) Memory.__stress.history.shift()

	// ④ 校准忙等: 精确顶到目标线 (getUsed 含意图计价, 与官方口径一致)
	const spin = () => {
		let s = 0
		for (let i = 0; i < 20000; i++) s += Math.sqrt(i)
		return s
	}
	let guard = 0
	while (Game.cpu.getUsed() < target && guard++ < 5000) spin()

	if (Game.time % 50 === 0) {
		console.log(`[stress] t=${Game.time} used=${Game.cpu.getUsed().toFixed(1)}/${limit} work=${work} guard=${guard}`)
	}
}
