// .sim.ts 场景静态检查: 模拟启动前的"编译器"层诊断。
// 目标: 把所有能静态发现的错误在引擎启动前指出, 并对可疑但不非法的写法给出警告。
//
// 输入为 wire 格式场景对象 (先经 toWireScenario 映射), 与 Rust 端 Scenario::validate 互补:
// Rust 校验是最后防线 (引擎加载时), 这里做得更早、更丰富、聚合展示。
//
// 注意: 常量表与 src/engine/constants.rs 保持同步 (标注了来源)。

// ── 常量 (镜像 constants.rs) ──────────────────────────────

// BODYPART_COST 键集合 (constants.rs:25)
const BODY_PARTS = new Set([
	'move',
	'work',
	'carry',
	'attack',
	'ranged_attack',
	'heal',
	'claim',
	'tough',
])

// 官方 BOOSTS 的 (部件 → 可用化合物) 键集合 (constants.rs BOOSTS)
const BOOSTS_BY_PART = {
	work: ['UO', 'UHO2', 'XUHO2', 'LH', 'LH2O', 'XLH2O', 'ZH', 'ZH2O', 'XZH2O', 'GH', 'GH2O', 'XGH2O'],
	attack: ['UH', 'UH2O', 'XUH2O'],
	ranged_attack: ['KO', 'KHO2', 'XKHO2'],
	heal: ['LO', 'LHO2', 'XLHO2'],
	carry: ['KH', 'KH2O', 'XKH2O'],
	move: ['ZO', 'ZHO2', 'XZHO2'],
	tough: ['GO', 'GHO2', 'XGHO2'],
}

// is_known_structure_type (constants.rs:31)
const STRUCTURE_TYPES = new Set([
	'spawn',
	'extension',
	'road',
	'constructedWall',
	'rampart',
	'link',
	'storage',
	'tower',
	'observer',
	'powerSpawn',
	'extractor',
	'lab',
	'terminal',
	'container',
	'nuker',
	'factory',
	'keeperLair',
	'powerBank',
	'portal',
	'controller',
	'invaderCore',
])

// controller_structure_limit (constants.rs:280), 索引即 RCL 0..8
const CONTROLLER_STRUCTURES = {
	spawn: [0, 1, 1, 1, 1, 1, 1, 2, 3],
	extension: [0, 0, 5, 10, 20, 30, 40, 50, 60],
	road: Array(9).fill(2500),
	constructedWall: Array(9).fill(0).fill(2500, 2),
	rampart: Array(9).fill(0).fill(2500, 2),
	link: [0, 0, 0, 0, 0, 2, 3, 4, 6],
	storage: [0, 0, 0, 0, 1, 1, 1, 1, 1],
	tower: [0, 0, 0, 1, 1, 2, 2, 3, 6],
	observer: [0, 0, 0, 0, 0, 0, 0, 0, 1],
	powerSpawn: [0, 0, 0, 0, 0, 0, 0, 0, 1],
	extractor: [0, 0, 0, 0, 0, 0, 1, 1, 1],
	lab: [0, 0, 0, 0, 0, 0, 3, 6, 10],
	terminal: [0, 0, 0, 0, 0, 0, 1, 1, 1],
	container: Array(9).fill(5),
	nuker: [0, 0, 0, 0, 0, 0, 0, 0, 1],
	factory: [0, 0, 0, 0, 0, 0, 0, 1, 1],
}

// 结构能量容量 (constants.rs SPAWN_ENERGY_CAPACITY / structure_energy_capacity / 各 CAPACITY)
const MAX_CREEP_SIZE = 50 // constants.rs:58
const STRUCTURE_ENERGY_CAPACITY = {
	spawn: () => 300,
	extension: (rcl) => (rcl <= 6 ? 50 : rcl === 7 ? 100 : 200), // extension_capacity
	link: () => 800,
	container: () => 2000,
	storage: () => 1_000_000,
	tower: () => 1000,
	terminal: () => 300_000,
}

const ROOM_NAME_RE = /^[WE]\d{1,3}[NS]\d{1,3}$/
const ASSERT_KINDS = new Set([
	'rclAtLeast',
	'rclReachedBy',
	'creepCountAtLeast',
	'cpuAvgBelow',
	'cpuP95Below',
	'noErrors',
])

// ── 诊断结构 ──────────────────────────────────────────────

/**
 * @returns {{ errors: Array<{path:string,message:string}>, warnings: Array<{path:string,message:string}> }}
 */
export function checkScenario(sim) {
	/** @type {Array<{path:string,message:string}>} */
	const errors = []
	/** @type {Array<{path:string,message:string}>} */
	const warnings = []
	const err = (path, message) => errors.push({ path, message })
	const warn = (path, message) => warnings.push({ path, message })

	if (typeof sim.name !== 'string' || !sim.name.trim()) {
		err('name', '不能为空')
	}
	if (!Array.isArray(sim.rooms) || sim.rooms.length === 0) {
		err('rooms', '不能为空')
		return { errors, warnings }
	}
	if (sim.ticks !== undefined) {
		if (!Number.isInteger(sim.ticks) || sim.ticks <= 0) err('ticks', `必须为正整数 (当前 ${sim.ticks})`)
	}
	if (sim.cpu && typeof sim.cpu === 'object') {
		if (sim.cpu.limit !== undefined && !(sim.cpu.limit > 0)) err('cpu.limit', `必须 > 0 (当前 ${sim.cpu.limit})`)
		if (sim.cpu.bucket !== undefined && sim.cpu.bucket < 0) err('cpu.bucket', `不能为负 (当前 ${sim.cpu.bucket})`)
		if (sim.cpu.tick_limit !== undefined && !(sim.cpu.tick_limit > 0)) {
			err('cpu.tick_limit', `覆盖值必须 > 0 (当前 ${sim.cpu.tick_limit})`)
		}
	}
	if (sim.globalResetEvery !== undefined && sim.globalResetEvery <= 0) {
		err('globalResetEvery', '必须 > 0 (不设置表示不重置)')
	}
	// npcs: invader 侵袭配置 (keeper/powerBank 由房间结构 keeperLair/powerBank 直接驱动)
	if (sim.npcs !== undefined) {
		if (!sim.npcs || typeof sim.npcs !== 'object' || Array.isArray(sim.npcs)) {
			err('npcs', '应为对象, 形如 { "invaders": { "every": 8000, "first": 3000, "lifetime": 15000, "squad": 3 } }')
		} else if (sim.npcs.invaders !== undefined) {
			const inv = sim.npcs.invaders
			if (!inv || typeof inv !== 'object' || Array.isArray(inv)) {
				err('npcs.invaders', '应为对象')
			} else {
				if (inv.every !== undefined && !(inv.every > 0)) err('npcs.invaders.every', `必须 > 0 (当前 ${inv.every})`)
				if (inv.first !== undefined && !(inv.first >= 0)) err('npcs.invaders.first', `不能为负 (当前 ${inv.first})`)
				if (inv.lifetime !== undefined && !(inv.lifetime > 0)) err('npcs.invaders.lifetime', `必须 > 0 (当前 ${inv.lifetime})`)
				if (inv.squad !== undefined && !(Number.isInteger(inv.squad) && inv.squad > 0)) {
					err('npcs.invaders.squad', `必须为正整数 (当前 ${inv.squad})`)
				}
			}
		}
	}
	// market: 初始 credits + 外部订单 (下单收 5% 手续费 / 运输能量费由接单方终端出)
	if (sim.market !== undefined) {
		if (!sim.market || typeof sim.market !== 'object' || Array.isArray(sim.market)) {
			err('market', '应为对象, 形如 { "credits": 10000, "orders": [{ "type": "sell", "resourceType": "energy", "roomName": "W1N1", "price": 0.5, "totalAmount": 1000 }] }')
		} else {
			if (sim.market.credits !== undefined && !(typeof sim.market.credits === 'number' && isFinite(sim.market.credits) && sim.market.credits >= 0)) {
				err('market.credits', `必须为非负有限数 (当前 ${sim.market.credits})`)
			}
			if (sim.market.orders !== undefined) {
				if (!Array.isArray(sim.market.orders)) {
					err('market.orders', '应为数组')
				} else {
					sim.market.orders.forEach((o, i) => {
						const at = `market.orders[${i}]`
						if (!o || typeof o !== 'object' || Array.isArray(o)) { err(at, '应为对象'); return }
						if (o.type !== 'buy' && o.type !== 'sell') err(`${at}.type`, `必须为 buy 或 sell (当前 ${o.type})`)
						if (typeof o.resourceType !== 'string') err(`${at}.resourceType`, '必须为字符串')
						if (typeof o.roomName !== 'string' || !/^(W|E)\d+(N|S)\d+$/.test(o.roomName)) err(`${at}.roomName`, `非法房名 (当前 ${o.roomName})`)
						if (!(typeof o.price === 'number' && isFinite(o.price) && o.price >= 0.001 && o.price <= 1e8)) err(`${at}.price`, `必须在 [0.001, 1e8] 内 (当前 ${o.price})`)
						if (!(Number.isInteger(o.totalAmount) && o.totalAmount > 0)) err(`${at}.totalAmount`, `必须为正整数 (当前 ${o.totalAmount})`)
					})
				}
			}
		}
	}
	if (sim.seed === 0) {
		warn('seed', 'seed=0: 地形生成确定, 但 Math.random 不播种 → 模拟不可复现')
	}
	if (sim.alwaysVisible) {
		warn('alwaysVisible', '调试开关 (全房间常可见) 开着, 可能掩盖 bot 视野逻辑问题')
	}
	if (typeof sim.memory === 'string' && sim.memory.trim()) {
		try {
			const parsed = JSON.parse(sim.memory)
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				warn('memory', '根节点不是普通对象, bot 侧 Memory.xxx 读写可能异常')
			}
		} catch {
			err('memory', '不是合法 JSON (引擎将回退为 {})')
		}
	}

	// ── 房间 ──
	const roomNames = new Set()
	const globalCreepNames = new Map() // name -> room
	for (const room of sim.rooms) {
		const ctx = `rooms[${room?.name ?? '?'}]`
		if (typeof room.name !== 'string' || !ROOM_NAME_RE.test(room.name)) {
			err(`${ctx}.name`, `非法房名 ${JSON.stringify(room.name)} (应形如 W5N8 / E12S3)`)
			// 不 continue: 房名非法也要继续检查该房间内容, 一次性聚合全部问题
		} else {
			if (roomNames.has(room.name)) err(`${ctx}.name`, '房间重复定义')
			roomNames.add(room.name)
		}

		// 地形
		const grid = terrainGrid(room.terrain)
		if (room.terrain?.kind === 'rows') {
			if (!Array.isArray(room.terrain.rows) || room.terrain.rows.length !== 50) {
				err(`${ctx}.terrain.rows`, `需要 50 行 (实际 ${room.terrain.rows?.length})`)
			} else {
				room.terrain.rows.forEach((row, i) => {
					if (typeof row !== 'string' || row.length !== 50 || !/^[.#~012]*$/.test(row)) {
						err(`${ctx}.terrain.rows[${i}]`, '需要 50 个字符, 且只含 . # ~ (或 0 1 2)')
					}
				})
			}
		}
		if (room.terrain?.kind === 'raw' && typeof room.terrain.data === 'string' && room.terrain.data.length !== 2500) {
			err(`${ctx}.terrain.raw`, `长度应为 2500, 实际 ${room.terrain.data.length}`)
		}

		// 坐标类字段
		const posCheck = (path, pos) => {
			if (!Array.isArray(pos) || pos.length !== 2 || !pos.every((n) => Number.isInteger(n) && n >= 0 && n <= 49)) {
				err(`${ctx}.${path}`, `坐标应为 [x,y] 且 0..49 (当前 ${JSON.stringify(pos)})`)
				return false
			}
			return true
		}

		// controller
		if (room.controller && typeof room.controller === 'object') {
			const c = room.controller
			if (!Number.isInteger(c.level) || c.level < 0 || c.level > 8) {
				err(`${ctx}.controller.level`, `应为 0..8 (当前 ${c.level})`)
			}
			if (c.pos) posCheck('controller.pos', c.pos)
			if (c.mine === false && c.level > 0) {
				warn(`${ctx}.controller`, 'mine=false 的中立控制器等级将被引擎强制为 0 (level 字段无效)')
			}
			if (c.safeModeAvailable !== undefined
				&& (!Number.isInteger(c.safeModeAvailable) || c.safeModeAvailable < 0)) {
				err(`${ctx}.controller.safeModeAvailable`, `应为非负整数 (当前 ${c.safeModeAvailable})`)
			}
		}

		// sources
		if (room.sources && typeof room.sources === 'object') {
			if (Array.isArray(room.sources.positions)) {
				if (room.sources.positions.length === 0) err(`${ctx}.sources.positions`, '不能为空')
				room.sources.positions.forEach((p, i) => {
					// 真实地图里 source 本来就在墙格上 (显式地形原样使用, 不开洞), 不再警告
					posCheck(`sources.positions[${i}]`, p)
				})
			} else if (room.sources.count !== undefined) {
				if (!Number.isInteger(room.sources.count) || room.sources.count < 1) {
					err(`${ctx}.sources.count`, `必须 ≥ 1 (当前 ${room.sources.count})`)
				} else if (room.sources.count > 3) {
					warn(`${ctx}.sources.count`, `数量 ${room.sources.count} 超过 3, 引擎将截断为 3`)
				}
			}
		}

		// mineral
		if (room.mineral && typeof room.mineral === 'object') {
			posCheck('mineral.pos', room.mineral.pos)
			const d = room.mineral.density
			if (d !== undefined && !(Number.isInteger(d) && d >= 1 && d <= 4)) err(`${ctx}.mineral.density`, `应为 1..4 (当前 ${d})`)
		}

		// structures
		const byPos = new Map()
		const typeCounts = new Map()
		const rcl = Number.isInteger(room.controller?.level) ? room.controller.level : 1
		for (const [i, st] of (room.structures ?? []).entries()) {
			const path = `${ctx}.structures[${i}]`
			if (!st || typeof st !== 'object') {
				err(path, '结构定义非法')
				continue
			}
			if (typeof st.type !== 'string' || !STRUCTURE_TYPES.has(st.type)) {
				err(`${path}.type`, `未知结构类型 ${JSON.stringify(st.type)}`)
			} else {
				typeCounts.set(st.type, (typeCounts.get(st.type) ?? 0) + 1)
			}
			if (posCheck(`structures[${i}].pos`, st.pos)) {
				const key = `${st.pos[0]},${st.pos[1]}`
				byPos.set(key, [...(byPos.get(key) ?? []), st.type])
				if (grid && grid[st.pos[1]]?.[st.pos[0]] === 1) {
					warn(`${path}.pos`, `(${st.pos[0]},${st.pos[1]}) 在墙上 (引擎会把该格改为平原)`)
				}
			}
			if (st.store && typeof st.store === 'object') {
				for (const [k, v] of Object.entries(st.store)) {
					if (!Number.isFinite(v) || v < 0) err(`${path}.store.${k}`, `应为非负数 (当前 ${v})`)
				}
			}
			if (st.energy !== undefined && typeof st.type === 'string' && STRUCTURE_ENERGY_CAPACITY[st.type]) {
				const cap = STRUCTURE_ENERGY_CAPACITY[st.type](rcl)
				if (st.energy > cap) warn(`${path}.energy`, `${st.energy} 超过 ${st.type} 容量 ${cap} (RCL${rcl})`)
			}
		}
		// 同格多结构: rampart/road 可与其他共存, 其余组合与官方"一格一结构"不符
		for (const [key, types] of byPos) {
			if (types.length > 1) {
				const real = types.filter((t) => t !== 'rampart' && t !== 'road')
				if (real.length > 1) warn(`${ctx}.structures`, `(${key}) 同时放置 ${types.join('+')} —— 官方一格一结构 (rampart/road 除外)`)
			}
		}
		// RCL 上限 (引擎会把超限结构标记 off 并排除出能量统计)
		for (const [type, n] of typeCounts) {
			// 中立/敌方结构 (keeperLair / powerBank / portal / controller / invaderCore) 无 RCL 上限
			if (!Object.prototype.hasOwnProperty.call(CONTROLLER_STRUCTURES, type)) continue
			const limit = CONTROLLER_STRUCTURES[type][Math.min(rcl, 8)] ?? 0
			if (n > limit) {
				warn(`${ctx}.structures`, `${type} ×${n} 超过 RCL${rcl} 上限 ${limit}, 超限结构将被引擎标记 off (如非刻意测试 off 行为请修正)`)
			}
		}
		// keeperLair 需要房间内有 source 才能绑定守卫目标 (无 source 则永不生成 keeper)
		if ((typeCounts.get('keeperLair') ?? 0) > 0) {
			const sourceCount = Array.isArray(room.sources?.positions)
				? room.sources.positions.length
				: (room.sources?.count ?? 0)
			if (sourceCount === 0) {
				warn(`${ctx}.structures`, '房间有 keeperLair 但没有 source, 引擎不会生成 keeper (lair 无守卫目标)')
			}
		}

		// creeps
		const roomCreepNames = new Set()
		for (const [i, cr] of (room.creeps ?? []).entries()) {
			const path = `${ctx}.creeps[${i}]`
			if (!cr || typeof cr !== 'object') {
				err(path, '单位定义非法')
				continue
			}
			if (typeof cr.name !== 'string' || !cr.name) {
				err(`${path}.name`, '不能为空')
			} else {
				if (roomCreepNames.has(cr.name)) err(`${path}.name`, `单位名 ${cr.name} 在房间内重复`)
				roomCreepNames.add(cr.name)
				const seenIn = globalCreepNames.get(cr.name)
				if (seenIn) err(`${path}.name`, `单位名 ${cr.name} 与 ${seenIn} 重复 (全游戏唯一)`)
				else globalCreepNames.set(cr.name, room.name)
			}
			if (!Array.isArray(cr.body) || cr.body.length === 0) {
				err(`${path}.body`, '身体不能为空')
			} else {
				if (cr.body.length > MAX_CREEP_SIZE) err(`${path}.body`, `部件数 ${cr.body.length} 超过 ${MAX_CREEP_SIZE}`)
				for (const [j, part] of cr.body.entries()) {
					// 部件可写成 "work" 或带强化的 {type: "work", boost: "UO"}
					const type = part && typeof part === 'object' ? part.type : part
					if (!BODY_PARTS.has(type)) {
						err(`${path}.body[${j}]`, `未知部件 ${JSON.stringify(part)}`)
					} else if (part && typeof part === 'object' && part.boost != null && !(BOOSTS_BY_PART[type] ?? []).includes(part.boost)) {
						err(`${path}.body[${j}].boost`, `${type} 部件不能用 ${JSON.stringify(part.boost)} 强化 (见官方 BOOSTS)`)
					}
				}
			}
			if (posCheck(`creeps[${i}].pos`, cr.pos) && grid && grid[cr.pos[1]]?.[cr.pos[0]] === 1) {
				warn(`${path}.pos`, `(${cr.pos[0]},${cr.pos[1]}) 在墙上 (引擎会把该格改为平原)`)
			}
			if (cr.store && typeof cr.store === 'object') {
				for (const [k, v] of Object.entries(cr.store)) {
					if (!Number.isFinite(v) || v < 0) err(`${path}.store.${k}`, `应为非负数 (当前 ${v})`)
				}
			}
			if (cr.enemy) warn(`${path}.enemy`, '存在敌方单位, FIND_HOSTILE_CREEPS 语义将被触发')
		}
	}

	// ── 房间连通性 (出口越房只发生在已定义的相邻房间之间) ──
	if (sim.rooms.length > 1 && sim.rooms.every((r) => ROOM_NAME_RE.test(r?.name ?? ''))) {
		const coords = (name) => {
			const m = /^([WE])(\d{1,3})([NS])(\d{1,3})$/.exec(name)
			const x = m[1] === 'E' ? +m[2] : -m[2]
			const y = m[3] === 'N' ? +m[4] : -m[4]
			return [x, y]
		}
		const names = sim.rooms.map((r) => r.name)
		const adj = (a, b) => {
			const [ax, ay] = coords(a)
			const [bx, by] = coords(b)
			return Math.abs(ax - bx) + Math.abs(ay - by) === 1
		}
		const start =
			sim.rooms.find((r) => r.structures?.some((s) => s?.type === 'spawn'))?.name ?? names[0]
		const visited = new Set([start])
		const queue = [start]
		while (queue.length) {
			const cur = queue.shift()
			for (const n of names) {
				if (!visited.has(n) && adj(cur, n)) {
					visited.add(n)
					queue.push(n)
				}
			}
		}
		const unreachable = names.filter((n) => !visited.has(n))
		if (unreachable.length) {
			warn('rooms', `房间 ${unreachable.join(', ')} 与 ${start} 不连通 (出口越房只在已定义的相邻房间间发生 —— 若非刻意, 检查房名拼写)`)
		}
	}

	// ── 断言 ──
	const effectiveTicks = sim.ticks ?? 1000
	for (const [i, a] of (sim.asserts ?? []).entries()) {
		const path = `asserts[${i}]`
		if (!a || typeof a !== 'object' || typeof a.kind !== 'string' || !ASSERT_KINDS.has(a.kind)) {
			err(`${path}.kind`, `未知断言类型 ${JSON.stringify(a?.kind)} (可用: ${[...ASSERT_KINDS].join(', ')})`)
			continue
		}
		if (a.kind === 'rclAtLeast' || a.kind === 'rclReachedBy') {
			if (!Number.isInteger(a.value) || a.value < 1 || a.value > 8) err(`${path}.value`, `RCL 应为 1..8 (当前 ${a.value})`)
		}
		if (a.kind === 'rclAtLeast' && a.room && !roomNames.has(a.room)) {
			err(`${path}.room`, `断言引用了不存在的房间 ${a.room}`)
		}
		if (a.kind === 'rclReachedBy') {
			if (!Number.isInteger(a.tick) || a.tick <= 0) {
				err(`${path}.tick`, `必须为正整数 (当前 ${a.tick})`)
			} else if (sim.ticks !== undefined && a.tick > sim.ticks) {
				err(`${path}.tick`, `要求 ≤ ${a.tick} tick 达成, 但场景只跑 ${sim.ticks} tick —— 断言不可能通过`)
			} else if (sim.ticks === undefined && a.tick > 1000) {
				warn(`${path}.tick`, `要求 ≤ ${a.tick} tick, 但场景未定义 ticks (引擎默认 1000) —— 断言不可能通过`)
			}
		}
		if ((a.kind === 'cpuAvgBelow' || a.kind === 'cpuP95Below') && !(a.value > 0)) {
			err(`${path}.value`, `必须 > 0 (当前 ${a.value})`)
		}
		if (a.kind === 'creepCountAtLeast' && (!Number.isInteger(a.value) || a.value < 1)) {
			err(`${path}.value`, `必须 ≥ 1 (当前 ${a.value})`)
		}
	}
	if (!sim.skip && (!Array.isArray(sim.asserts) || sim.asserts.length === 0)) {
		warn('asserts', '场景没有任何断言 —— 跑完即通过, 无法防止行为回退 (建议至少加 noErrors)')
	}
	if (sim.skip) {
		warn('skip', '用例被跳过')
	}

	return { errors, warnings }
}

/// rows/raw 地形 → 0/1/2 网格 (仅显式地形可静态判定; flat/generated 返回 null)
function terrainGrid(terrain) {
	if (!terrain || typeof terrain !== 'object') return null
	if (terrain.kind === 'rows' && Array.isArray(terrain.rows) && terrain.rows.length === 50) {
		return terrain.rows.map((row) =>
			[...row].map((ch) => (ch === '#' || ch === '1' ? 1 : ch === '~' || ch === '2' ? 2 : 0))
		)
	}
	if (terrain.kind === 'raw' && typeof terrain.data === 'string' && terrain.data.length === 2500) {
		const grid = []
		for (let y = 0; y < 50; y++) {
			const row = []
			for (let x = 0; x < 50; x++) row.push(Number(terrain.data[y * 50 + x]))
			grid.push(row)
		}
		return grid
	}
	return null
}
