// 高密度意图 bot: 40 个单位每 tick 全部提交 move (1+..%8) + 采集尝试
// —— 模拟真实 bot 的意图密度与引擎移动仲裁负载
exports.loop = function () {
	let n = 0
	for (const name in Game.creeps) {
		const c = Game.creeps[name]
		const dir = 1 + ((Game.time + n) % 8)
		c.move(dir)
		c.harvest(c.pos.findClosestByRange(FIND_SOURCES) || c)
		n++
	}
}
