// 战斗演示 bot: 激活安全模式 + 发射核弹 + 读取安全模式字段 / 在飞核弹。
// 配合 examples/combat.sim.ts 使用: screeps-sim test examples/combat.sim.ts --verbose
exports.loop = function () {
	const room = Game.rooms['W5N8'];
	if (!room) return;
	const controller = room.controller;

	if (Game.time === 1 && controller && controller.my) {
		// 消耗一次激活次数: 20000 tick 内非房主攻击被拦截
		controller.activateSafeMode();
	}
	if (Game.time === 2) {
		const nuker = room
			.find(FIND_MY_STRUCTURES, { filter: (s) => s.structureType === STRUCTURE_NUKER })[0];
		// 目标坐标可用任意房间的 RoomPosition; 核弹 50000 tick 后落地
		if (nuker) nuker.launchNuke(new RoomPosition(10, 10, 'W5N8'));
	}

	const state = {
		t: Game.time,
		safeMode: controller ? controller.safeMode : null,
		safeModeAvailable: controller ? controller.safeModeAvailable : 0,
		safeModeCooldown: controller ? controller.safeModeCooldown : null,
		upgradeBlocked: controller ? controller.upgradeBlocked : 0,
		nukes: room.find(FIND_NUKES).length,
	};
	Memory.combat = state;
	if (Game.time % 50 === 0) console.log(JSON.stringify(state));
};
