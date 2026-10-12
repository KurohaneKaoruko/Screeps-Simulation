// .sim.ts 的 TS 友好 camelCase 字段 → Rust 引擎 wire JSON (snake_case) 的映射。
// 未知字段原样透传 —— 直接写 wire 字段名 (如已有 scenarios/*.json 的风格) 也有效。
//
// 需要映射的字段 (其余同名直传):
//   cpu.tickLimit        → cpu.tick_limit
//   alwaysVisible        → always_visible
//   deterministicRandom  → deterministic_random
//   mineral.type         → mineral.mineral_type (引擎同时 alias 接受 mineralType)
//   globalResetEvery     → 原样 (引擎 wire 键本来就是 camelCase, serde rename)
//   structures[].store / structures[].level / mineral.density / creeps[].body 的 {type, boost} → 同名直传
export function toWireScenario(sim) {
	const wire = { ...sim }
	delete wire.__simCase
	// runner 专用字段, 不进引擎
	delete wire.bundle
	delete wire.skip

	if (wire.cpu && typeof wire.cpu === 'object') {
		const cpu = { ...wire.cpu }
		if (cpu.tickLimit !== undefined) {
			cpu.tick_limit = cpu.tickLimit
			delete cpu.tickLimit
		}
		wire.cpu = cpu
	}
	if (wire.alwaysVisible !== undefined) {
		wire.always_visible = wire.alwaysVisible
		delete wire.alwaysVisible
	}
	if (wire.deterministicRandom !== undefined) {
		wire.deterministic_random = wire.deterministicRandom
		delete wire.deterministicRandom
	}
	if (Array.isArray(wire.rooms)) {
		wire.rooms = wire.rooms.map((room) => {
			if (!room || typeof room !== 'object' || !room.mineral) return room
			const mineral = { ...room.mineral }
			if (mineral.type !== undefined) {
				mineral.mineral_type = mineral.type
				delete mineral.type
			}
			return { ...room, mineral }
		})
	}
	return wire
}
