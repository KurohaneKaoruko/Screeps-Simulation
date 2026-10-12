// NPC 演示 bot: 读取 keeperLair / powerBank / invader 的 API 字段。
// 配合 examples/npc.sim.ts 使用: screeps-sim test examples/npc.sim.ts --verbose
exports.loop = function () {
	const room = Game.rooms['W5N8'];
	if (!room) return;

	const structures = room.find(FIND_STRUCTURES);
	const lair = structures.find((s) => s.structureType === 'keeperLair');
	const bank = structures.find((s) => s.structureType === 'powerBank');
	const hostiles = room.find(FIND_HOSTILE_CREEPS);
	const keepers = hostiles.filter((c) => c.name.indexOf('Keeper') === 0);
	const invaders = hostiles.filter((c) => c.name.indexOf('Invader') === 0);

	const state = {
		t: Game.time,
		ticksToSpawn: lair ? lair.ticksToSpawn : null,
		power: bank ? bank.power : null,
		ticksToDecay: bank ? bank.ticksToDecay : null,
		keepers: keepers.length,
		invaders: invaders.length,
	};

	// Memory 里保留最新一份, 便于断言/调试; 每 100 tick 打印一次避免刷屏
	Memory.npc = state;
	if (Game.time % 100 === 0) console.log(JSON.stringify(state));
};
