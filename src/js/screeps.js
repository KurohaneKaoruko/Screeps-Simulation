/**
 * Screeps API shim: 在 V8 沙箱内为 bot bundle 提供全套游戏全局对象。
 * 每个游戏 tick: Rust 注入 __SNAPSHOT_JSON / __MEMORY_JSON → __runTick() 重建 Game →
 * 调用 bot loop() → 收集意图与控制台输出 → 返回 JSON 结果。
 *
 * 结算规则与返回码对齐官方 runtime (@screeps/engine src/game/*)。
 */
"use strict";

// ═══════════════════════════ 常量 ═══════════════════════════

const C = {
	OK: 0,
	ERR_NOT_OWNER: -1,
	ERR_NO_PATH: -2,
	ERR_NAME_EXISTS: -3,
	ERR_BUSY: -4,
	ERR_NOT_FOUND: -5,
	ERR_NOT_ENOUGH_RESOURCES: -6,
	ERR_NOT_ENOUGH_ENERGY: -6,
	ERR_INVALID_TARGET: -7,
	ERR_FULL: -8,
	ERR_NOT_IN_RANGE: -9,
	ERR_INVALID_ARGS: -10,
	ERR_TIRED: -11,
	ERR_NO_BODYPART: -12,
	ERR_NOT_ENOUGH_EXTENSIONS: -13,
	ERR_RCL_NOT_ENOUGH: -14,
	ERR_GCL_NOT_ENOUGH: -15,

	FIND_EXIT_TOP: 1,
	FIND_EXIT_RIGHT: 3,
	FIND_EXIT_BOTTOM: 5,
	FIND_EXIT_LEFT: 7,
	FIND_EXIT: 10,
	FIND_CREEPS: 101,
	FIND_MY_CREEPS: 102,
	FIND_HOSTILE_CREEPS: 103,
	FIND_SOURCES_ACTIVE: 104,
	FIND_SOURCES: 105,
	FIND_DROPPED_RESOURCES: 106,
	FIND_STRUCTURES: 107,
	FIND_MY_STRUCTURES: 108,
	FIND_HOSTILE_STRUCTURES: 109,
	FIND_FLAGS: 110,
	FIND_CONSTRUCTION_SITES: 111,
	FIND_MY_SPAWNS: 112,
	FIND_HOSTILE_SPAWNS: 113,
	FIND_MY_CONSTRUCTION_SITES: 114,
	FIND_HOSTILE_CONSTRUCTION_SITES: 115,
	FIND_MINERALS: 116,
	FIND_NUKES: 117,
	FIND_TOMBSTONES: 118,
	FIND_POWER_CREEPS: 119,
	FIND_MY_POWER_CREEPS: 120,
	FIND_HOSTILE_POWER_CREEPS: 121,
	FIND_DEPOSITS: 122,
	FIND_RUINS: 123,

	LOOK_CREEPS: "creep",
	LOOK_ENERGY: "energy",
	LOOK_RESOURCES: "resource",
	LOOK_SOURCES: "source",
	LOOK_MINERALS: "mineral",
	LOOK_DEPOSITS: "deposit",
	LOOK_STRUCTURES: "structure",
	LOOK_FLAGS: "flag",
	LOOK_CONSTRUCTION_SITES: "constructionSite",
	LOOK_NUKES: "nuke",
	LOOK_TERRAIN: "terrain",
	LOOK_TOMBSTONES: "tombstone",
	LOOK_POWER_CREEPS: "powerCreep",
	LOOK_RUINS: "ruin",

	TOP: 1,
	TOP_RIGHT: 2,
	RIGHT: 3,
	BOTTOM_RIGHT: 4,
	BOTTOM: 5,
	BOTTOM_LEFT: 6,
	LEFT: 7,
	TOP_LEFT: 8,

	MOVE: "move",
	WORK: "work",
	CARRY: "carry",
	ATTACK: "attack",
	RANGED_ATTACK: "ranged_attack",
	TOUGH: "tough",
	HEAL: "heal",
	CLAIM: "claim",

	COLOR_RED: 1, COLOR_PURPLE: 2, COLOR_BLUE: 3, COLOR_CYAN: 4, COLOR_GREEN: 5,
	COLOR_YELLOW: 6, COLOR_ORANGE: 7, COLOR_BROWN: 8, COLOR_GREY: 9, COLOR_WHITE: 10,

	CREEP_LIFE_TIME: 1500,
	CREEP_CLAIM_LIFE_TIME: 600,
	CREEP_SPAWN_TIME: 3,
	CREEP_CORPSE_RATE: 0.2,
	MAX_CREEP_SIZE: 50,
	MAX_CONSTRUCTION_SITES: 100,

	BODYPART_COST: { move: 50, work: 100, carry: 50, attack: 80, ranged_attack: 150, tough: 10, heal: 250, claim: 600 },

	BODYPARTS_ALL: ["move", "work", "carry", "attack", "ranged_attack", "tough", "heal", "claim"],
	RESOURCES_ALL: ["energy", ...["H", "O", "U", "K", "L", "Z", "X", "G"].flatMap((b) => [b, `${b}O`, `${b}H`, `${b}L`, `${b}K`, `${b}Z`, `${b}U`].slice(0, 4))],

	RESOURCE_ENERGY: "energy",
	RESOURCE_POWER: "power",
	RESOURCE_OPS: "ops",

	STRUCTURE_SPAWN: "spawn",
	STRUCTURE_EXTENSION: "extension",
	STRUCTURE_ROAD: "road",
	STRUCTURE_WALL: "constructedWall",
	STRUCTURE_RAMPART: "rampart",
	STRUCTURE_KEEPER_LAIR: "keeperLair",
	STRUCTURE_PORTAL: "portal",
	STRUCTURE_CONTROLLER: "controller",
	STRUCTURE_LINK: "link",
	STRUCTURE_STORAGE: "storage",
	STRUCTURE_TOWER: "tower",
	STRUCTURE_OBSERVER: "observer",
	STRUCTURE_POWER_BANK: "powerBank",
	STRUCTURE_POWER_SPAWN: "powerSpawn",
	STRUCTURE_EXTRACTOR: "extractor",
	STRUCTURE_LAB: "lab",
	STRUCTURE_TERMINAL: "terminal",
	STRUCTURE_CONTAINER: "container",
	STRUCTURE_NUKER: "nuker",
	STRUCTURE_FACTORY: "factory",
	STRUCTURE_INVADER_CORE: "invaderCore",

	CONSTRUCTION_COST: {
		spawn: 15000, extension: 3000, road: 300, constructedWall: 1, rampart: 1,
		link: 5000, storage: 30000, tower: 5000, observer: 8000, powerSpawn: 100000,
		extractor: 5000, lab: 50000, terminal: 100000, container: 5000, nuker: 100000, factory: 100000,
	},
	CONSTRUCTION_COST_ROAD_SWAMP_RATIO: 5,
	CONSTRUCTION_COST_ROAD_WALL_RATIO: 150,

	CONTROLLER_LEVELS: { 1: 200, 2: 45000, 3: 135000, 4: 405000, 5: 1215000, 6: 3645000, 7: 10935000 },
	CONTROLLER_DOWNGRADE: { 1: 20000, 2: 10000, 3: 20000, 4: 40000, 5: 80000, 6: 120000, 7: 150000, 8: 200000 },
	CONTROLLER_DOWNGRADE_RESTORE: 100,
	CONTROLLER_DOWNGRADE_SAFEMODE_THRESHOLD: 5000,
	CONTROLLER_CLAIM_DOWNGRADE: 300,
	CONTROLLER_RESERVE: 1,
	CONTROLLER_RESERVE_MAX: 5000,
	CONTROLLER_MAX_UPGRADE_PER_TICK: 15,
	CONTROLLER_ATTACK_BLOCKED_UPGRADE: 1000,
	CONTROLLER_NUKE_BLOCKED_UPGRADE: 200,

	GCL_POW: 2.4,
	GCL_MULTIPLY: 1000000,

	TERRAIN_MASK_WALL: 1,
	TERRAIN_MASK_SWAMP: 2,
	TERRAIN_MASK_LAVA: 4,

	OBSTACLE_OBJECT_TYPES: [
		"spawn", "creep", "powerCreep", "source", "mineral", "deposit", "controller",
		"constructedWall", "extension", "link", "storage", "tower", "observer",
		"powerSpawn", "powerBank", "lab", "terminal", "nuker", "factory", "invaderCore",
	],
};
for (const k of Object.keys(C)) {
	globalThis[k] = C[k];
}
globalThis.RESOURCES_ALL = C.RESOURCES_ALL;

// ═══════════════════════════ 最小 lodash 兼容 ═══════════════════════════
// 部分服务器提供全局 _; bot 的移动模块 (RosmarinBot 血统) 使用了少量方法。

var _ = {
	any(arr, pred) { return Array.isArray(arr) ? arr.some(pred) : false; },
	all(arr, pred) { return Array.isArray(arr) ? arr.every(pred) : true; },
	find(arr, pred) { return Array.isArray(arr) ? arr.find(pred) : undefined; },
	filter(arr, pred) {
		if (!Array.isArray(arr)) return [];
		if (typeof pred === "function") return arr.filter(pred);
		if (pred && typeof pred === "object") return arr.filter((o) => Object.keys(pred).every((k) => o && o[k] === pred[k]));
		return arr.filter(Boolean);
	},
	map(arr, fn) { return Array.isArray(arr) ? arr.map(fn) : []; },
	forEach(arr, fn) { if (Array.isArray(arr)) arr.forEach(fn); },
	sum(arr, key) {
		if (!Array.isArray(arr)) return 0;
		return arr.reduce((acc, i) => acc + (key ? (i ? i[key] : 0) : i), 0);
	},
	isNumber(v) { return typeof v === "number"; },
	isUndefined(v) { return v === undefined; },
	isString(v) { return typeof v === "string"; },
	isObject(v) { return v !== null && typeof v === "object"; },
	isArray(v) { return Array.isArray(v); },
	contains(arr, v) { return Array.isArray(arr) && arr.includes(v); },
	uniq(arr) { return Array.isArray(arr) ? [...new Set(arr)] : []; },
	clone(obj) { return obj == null ? obj : (Array.isArray(obj) ? obj.slice() : { ...obj }); },
	cloneDeep(obj) { return obj == null ? obj : JSON.parse(JSON.stringify(obj)); },
	defaults(obj, ...srcs) {
		for (const src of srcs) {
			for (const k of Object.keys(src || {})) if (obj[k] === undefined) obj[k] = src[k];
		}
		return obj;
	},
	extend(obj, ...srcs) { return Object.assign(obj, ...srcs); },
	take(arr, n) { return Array.isArray(arr) ? arr.slice(0, n) : []; },
	sortBy(arr, key) {
		return Array.isArray(arr) ? [...arr].sort((a, b) => (a[key] > b[key] ? 1 : a[key] < b[key] ? -1 : 0)) : [];
	},
	indexOf(arr, v) { return Array.isArray(arr) ? arr.indexOf(v) : -1; },
	last(arr) { return Array.isArray(arr) && arr.length ? arr[arr.length - 1] : undefined; },
	size(v) { return v == null ? 0 : (Array.isArray(v) || typeof v === "string" ? v.length : Object.keys(v).length); },
};
globalThis._ = _;

// ═══════════════════════════ tick 间状态 ═══════════════════════════

/** 地形缓存: roomName → Uint8Array(2500), 整个进程只解析一次 */
const terrainCache = new Map();

function terrainOf(roomName) {
	let t = terrainCache.get(roomName);
	if (!t) {
		const raw = (__W.terrain || {})[roomName];
		t = new Uint8Array(2500);
		if (raw) for (let i = 0; i < 2500 && i < raw.length; i++) t[i] = +raw[i];
		terrainCache.set(roomName, t);
	}
	return t;
}

/** 当前 tick 的世界快照 (已 parse) */
let __W = null;
/** 意图收集: {o, n, d} */
let __intents = [];
/** 控制台缓冲 */
let __consoleLines = [];

function intent(objId, name, data) {
	__intents.push({ o: typeof objId === "string" ? objId : String(objId), n: name, d: data });
}

// ═══════════════════════════ 基础工具 ═══════════════════════════

function getDirection(dx, dy) {
	const adx = Math.abs(dx), ady = Math.abs(dy);
	if (adx > ady * 2) return dx > 0 ? C.RIGHT : C.LEFT;
	if (ady > adx * 2) return dy > 0 ? C.BOTTOM : C.TOP;
	if (dx > 0 && dy > 0) return C.BOTTOM_RIGHT;
	if (dx > 0 && dy < 0) return C.TOP_RIGHT;
	if (dx < 0 && dy > 0) return C.BOTTOM_LEFT;
	if (dx < 0 && dy < 0) return C.TOP_LEFT;
	return C.TOP;
}

const OFFSETS = [, [0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0], [-1, -1]];

/** 房名 ⇄ 世界房坐标 (对齐官方 roomNameToXY) */
function roomNameToXY(name) {
	let xx = parseInt(name.substr(1), 10);
	let verticalPos = 2;
	if (xx >= 100) verticalPos = 4;
	else if (xx >= 10) verticalPos = 3;
	let yy = parseInt(name.substr(verticalPos + 1), 10);
	if (name.charAt(0) === "W" || name.charAt(0) === "w") xx = -xx - 1;
	if (name.charAt(verticalPos) === "N" || name.charAt(verticalPos) === "n") yy = -yy - 1;
	return [xx, yy];
}
function xyToRoomName(x, y) {
	const [hx, xx] = x < 0 ? ["W", -x - 1] : ["E", x];
	const [hy, yy] = y < 0 ? ["N", -y - 1] : ["S", y];
	return `${hx}${xx}${hy}${yy}`;
}
function validRoomName(name) {
	return /^([WE])(\d+)([NS])(\d+)$/.test(name) && (__W ? __W.roomNames.includes(name) : true);
}

// ═══════════════════════════ RoomPosition ═══════════════════════════

class RoomPosition {
	constructor(x, y, roomName) {
		this.x = x;
		this.y = y;
		this.roomName = roomName;
	}
	getRangeTo(target) {
		const pos = target && target.pos ? target.pos : target;
		if (!pos || !this.roomName || pos.roomName !== this.roomName) {
			if (!pos) return NaN;
			return Math.abs(this.x - pos.x) > 25 ? Infinity : Math.max(Math.abs(this.x - pos.x), Math.abs(this.y - pos.y));
		}
		return Math.max(Math.abs(this.x - pos.x), Math.abs(this.y - pos.y));
	}
	inRangeTo(target, range) {
		return this.getRangeTo(target) <= range;
	}
	isEqualTo(target, y) {
		const pos = typeof y === "number" ? { x: target, y, roomName: this.roomName } : (target && target.pos ? target.pos : target);
		return this.x === pos.x && this.y === pos.y && this.roomName === pos.roomName;
	}
	isNearTo(target) {
		return this.getRangeTo(target) === 1;
	}
	getDirectionTo(target) {
		const pos = target && target.pos ? target.pos : target;
		return getDirection(pos.x - this.x, pos.y - this.y);
	}
	lookFor(type) {
		const room = Game.rooms[this.roomName];
		if (!room) return [];
		return room.lookForAt(type, this.x, this.y);
	}
	look() {
		const room = Game.rooms[this.roomName];
		if (!room) return [];
		return room.lookAt(this.x, this.y);
	}
	createConstructionSite(structureType, name) {
		const room = Game.rooms[this.roomName];
		if (!room) return C.ERR_INVALID_ARGS;
		return room.createConstructionSite(this.x, this.y, structureType, name);
	}
	createFlag(name, color, secondaryColor) {
		const room = Game.rooms[this.roomName];
		if (!room) return C.ERR_INVALID_ARGS;
		return room.createFlag(this.x, this.y, name, color, secondaryColor);
	}
	findClosestByRange(type, opts) {
		return findClosestImpl(this, type, opts, "range");
	}
	findClosestByPath(type, opts) {
		return findClosestImpl(this, type, opts, "path");
	}
	findInRange(type, range, opts) {
		const objects = collectFind(this.roomName, type, opts);
		return objects.filter((o) => this.getRangeTo(o.pos || o) <= range);
	}
	findPathTo(x, y, opts) {
		let target = x;
		if (typeof y === "number") target = new RoomPosition(x, y, this.roomName);
		else if (y && typeof y === "object" && !opts) opts = y;
		return roomFindPathImpl(this.roomName, this, target.pos || target, opts);
	}
	toString() {
		return `[room ${this.roomName} pos ${this.x},${this.y}]`;
	}
}

// ═══════════════════════════ Store ═══════════════════════════

class Store {
	constructor(contents, capacity, capacities) {
		this.__capacity = capacity || 0;
		this.__capacities = capacities || null;
		this.__contents = contents || {};
		for (const k of Object.keys(this.__contents)) this[k] = this.__contents[k];
	}
	getCapacity(resource) {
		const r = resource || "energy";
		if (this.__capacities) return this.__capacities[r] ?? 0;
		return this.__capacity;
	}
	getUsedCapacity(resource) {
		const r = resource || "energy";
		if (this.__capacities) return this[r] || 0;
		let sum = 0;
		for (const k of Object.keys(this.__contents)) sum += this.__contents[k];
		return sum;
	}
	getFreeCapacity(resource) {
		return this.getCapacity(resource) - this.getUsedCapacity(resource);
	}
}

function makeStore(raw, storeCapacity, energyCapacity) {
	const contents = raw || {};
	const caps = storeCapacity == null ? null : { energy: energyCapacity ?? storeCapacity };
	return new Store(contents, storeCapacity ?? 0, caps);
}

// ═══════════════════════════ RoomObject 层 ═══════════════════════════

class RoomObject {
	constructor(raw) {
		this.id = raw.id;
		this.pos = new RoomPosition(raw.x, raw.y, raw.roomName);
		this.__raw = raw;
	}
	get room() {
		return Game.rooms[this.pos.roomName];
	}
	get effects() {
		return [];
	}
}

class Creep extends RoomObject {
	constructor(raw) {
		super(raw);
		this.name = raw.name;
		this.my = raw.my !== false;
		this.owner = { username: "Nexus" };
		this.body = raw.body;
		this.fatigue = raw.fatigue;
		this.spawning = !!raw.spawning;
		this.ticksToLive = raw.ticksToLive;
		this.hits = raw.hits;
		this.hitsMax = raw.hitsMax;
		this.__storeRaw = raw.store;
		this.__storeCap = raw.storeCapacity;
		this.saying = undefined;
	}
	get store() {
		if (!this.__storeObj) this.__storeObj = new Store(this.__storeRaw, this.__storeCap, null);
		return this.__storeObj;
	}
	get carry() {
		return this.store;
	}
	get carryCapacity() {
		return this.__storeCap;
	}
	get memory() {
		if (!Memory.creeps) Memory.creeps = {};
		if (!Memory.creeps[this.name]) Memory.creeps[this.name] = {};
		return Memory.creeps[this.name];
	}
	set memory(v) {
		if (!Memory.creeps) Memory.creeps = {};
		Memory.creeps[this.name] = v;
	}
	getActiveBodyparts(type) {
		let n = 0;
		for (let i = this.body.length - 1; i >= 0; i--) {
			if (this.body[i].hits <= 0) break;
			if (this.body[i].type === type) n++;
		}
		return n;
	}
	move(direction) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("move")) return C.ERR_NO_BODYPART;
		direction = +direction;
		if (!direction || direction < 1 || direction > 8) return C.ERR_INVALID_ARGS;
		intent(this.id, "move", { direction });
		return C.OK;
	}
	moveByPath(path) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (Array.isArray(path) && path.length > 0) {
			if (path[0] instanceof RoomPosition) {
				let idx = path.findIndex((p) => p.isEqualTo(this.pos));
				if (idx === -1 && !path[0].isNearTo(this.pos)) return C.ERR_NOT_FOUND;
				if (idx >= 0) idx++;
				if (idx >= path.length) return C.ERR_NOT_FOUND;
				return this.move(this.pos.getDirectionTo(path[idx]));
			}
			const cur = path.find((i) => i.x - i.dx === this.pos.x && i.y - i.dy === this.pos.y);
			if (!cur) return C.ERR_NOT_FOUND;
			return this.move(cur.direction);
		}
		return this.move(path); // 序列化路径 = 单方向
	}
	moveTo(firstArg, secondArg, opts) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (typeof firstArg === "object" && firstArg !== null) opts = { ...secondArg };
		opts = opts || {};
		if (this.fatigue > 0 && !opts.visualizePathStyle) return C.ERR_TIRED;
		if (!this.getActiveBodyparts("move")) return C.ERR_NO_BODYPART;

		let x, y, roomName;
		if (typeof secondArg === "number") [x, y] = [firstArg, secondArg];
		else {
			const pos = firstArg && firstArg.pos ? firstArg.pos : firstArg;
			if (!(pos instanceof RoomPosition)) return C.ERR_INVALID_TARGET;
			x = pos.x; y = pos.y; roomName = pos.roomName;
		}
		roomName = roomName || this.pos.roomName;
		const targetPos = new RoomPosition(x, y, roomName);
		if (this.pos.x === x && this.pos.y === y && this.pos.roomName === roomName) return C.OK;
		if (opts.reusePath === undefined) opts.reusePath = 5;
		if (opts.serializeMemory === undefined) opts.serializeMemory = true;

		if (opts.reusePath && this.memory && typeof this.memory === "object" && this.memory._move) {
			const _move = this.memory._move;
			if (Game.time > _move.time + parseInt(opts.reusePath) || _move.room !== this.pos.roomName) {
				delete this.memory._move;
			} else if (_move.dest.room === roomName && _move.dest.x === x && _move.dest.y === y) {
				let path = typeof _move.path === "string" ? deserializePath(_move.path) : _move.path;
				const idx = path.findIndex((p) => p.x === this.pos.x && p.y === this.pos.y && p.roomName === this.pos.roomName);
				if (idx !== -1) {
					path = path.slice(idx + 1);
					_move.path = opts.serializeMemory ? serializePath(path) : path;
				}
				if (path.length === 0) {
					return this.pos.isNearTo(targetPos) ? C.OK : C.ERR_NO_PATH;
				}
				const result = this.moveByPath(path);
				if (result === C.OK) return C.OK;
			}
		}
		if (opts.noPathFinding) return C.ERR_NOT_FOUND;

		const path = this.pos.findPathTo(targetPos, opts);
		if (opts.reusePath && this.memory && typeof this.memory === "object") {
			this.memory._move = {
				dest: { x, y, room: roomName },
				time: Game.time,
				path: opts.serializeMemory ? serializePath(path) : [...path],
				room: this.pos.roomName,
			};
		}
		if (path.length === 0) return C.ERR_NO_PATH;
		return this.move(path[0].direction);
	}
	harvest(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Source)) return C.ERR_INVALID_TARGET;
		if (!target.energy) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "harvest", { id: target.id });
		return C.OK;
	}
	transfer(target, resourceType, amount) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!C.RESOURCES_ALL.includes(resourceType)) return C.ERR_INVALID_ARGS;
		const store = this.__storeRaw;
		if (!store || !store[resourceType]) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (amount === undefined) amount = store[resourceType];
		if (amount < 0) return C.ERR_INVALID_ARGS;
		if (!target || !target.id || target.spawning) return C.ERR_INVALID_TARGET;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		if (!(target instanceof Creep) && !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (target instanceof Structure) {
			const cap = target.__energyCapacity ?? 0;
			const stored = target.__storeRaw ? target.__storeRaw[resourceType] || 0 : 0;
			if (resourceType === "energy" && cap && stored >= cap) return C.ERR_FULL;
		}
		intent(this.id, "transfer", { id: target.id, resourceType, amount });
		return C.OK;
	}
	withdraw(target, resourceType, amount) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!C.RESOURCES_ALL.includes(resourceType)) return C.ERR_INVALID_ARGS;
		if (!target || !target.id || !(target instanceof RoomObject)) return C.ERR_INVALID_TARGET;
		const available = target.__storeRaw ? target.__storeRaw[resourceType] || 0 : 0;
		if (amount === undefined) amount = available;
		if (!available) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "withdraw", { id: target.id, resourceType, amount });
		return C.OK;
	}
	pickup(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !(target instanceof Resource)) return C.ERR_INVALID_TARGET;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		if (this.store.getFreeCapacity() <= 0) return C.ERR_FULL;
		intent(this.id, "pickup", { id: target.id });
		return C.OK;
	}
	drop(resourceType, amount) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!C.RESOURCES_ALL.includes(resourceType)) return C.ERR_INVALID_ARGS;
		const store = this.__storeRaw;
		if (!store || !store[resourceType]) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (amount === undefined) amount = store[resourceType];
		if (store[resourceType] < amount) return C.ERR_NOT_ENOUGH_ENERGY;
		intent(this.id, "drop", { amount, resourceType });
		return C.OK;
	}
	build(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!target || !(target instanceof ConstructionSite)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target.pos, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "build", { id: target.id });
		return C.OK;
	}
	repair(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!target || !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (target.hits >= target.hitsMax) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target.pos, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "repair", { id: target.id });
		return C.OK;
	}
	upgradeController(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!target || !(target instanceof StructureController)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target.pos, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "upgradeController", { id: target.id });
		return C.OK;
	}
	say(message, toPublic) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (typeof message !== "string") return C.ERR_INVALID_ARGS;
		intent(this.id, "say", { message: message.substr(0, 10), isPublic: !!toPublic });
		return C.OK;
	}
	suicide() {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		intent(this.id, "suicide", {});
		return C.OK;
	}
	toString() {
		return `[creep ${this.name}]`;
	}
}

class Source extends RoomObject {
	constructor(raw) {
		super(raw);
		this.energy = raw.energy;
		this.energyCapacity = raw.energyCapacity;
		this.ticksToRegeneration = raw.ticksToRegeneration;
	}
	toString() {
		return `[source ${this.id}]`;
	}
}

class Resource extends RoomObject {
	constructor(raw) {
		super(raw);
		this.resourceType = "energy";
		this.amount = raw.energy;
	}
}

class ConstructionSite extends RoomObject {
	constructor(raw) {
		super(raw);
		this.structureType = raw.structureType;
		this.progress = raw.progress;
		this.progressTotal = raw.progressTotal;
		this.my = raw.my !== false;
		this.owner = { username: "Nexus" };
	}
	remove() {
		if (!this.my) return C.ERR_NOT_OWNER;
		intent("room", "removeConstructionSite", { id: this.id, room: this.pos.roomName });
		return C.OK;
	}
	toString() {
		return `[constructionSite ${this.structureType} ${this.pos.x},${this.pos.y}]`;
	}
}

class Tombstone extends RoomObject {
	constructor(raw) {
		super(raw);
		this.store = new Store(raw.store, Infinity, null);
		this.ticksToDecay = raw.ticksToDecay;
		this.creep = raw.creepName ? { name: raw.creepName } : null;
		this.deathTime = 0;
	}
}

class Structure extends RoomObject {
	constructor(raw) {
		super(raw);
		this.structureType = raw.structureType;
		this.hits = raw.hits;
		this.hitsMax = raw.hitsMax;
		this.my = raw.my !== false;
		this.__storeRaw = raw.store;
		this.__storeCap = raw.storeCapacity;
		this.__energyCapacity = raw.energyCapacity;
	}
	get store() {
		if (!this.__storeObj && this.__storeRaw) this.__storeObj = new Store(this.__storeRaw, this.__storeCap, null);
		return this.__storeObj;
	}
	isActive() {
		return true;
	}
	destroy() {
		if (!this.my) return C.ERR_NOT_OWNER;
		intent("room", "destroyStructure", { id: this.id, room: this.pos.roomName });
		return C.OK;
	}
	notifyWhenAttacked() {
		return C.OK;
	}
	toString() {
		return `[structure ${this.structureType} #${this.id}]`;
	}
}

class OwnedStructure extends Structure { }
class StructureSpawn extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.name = raw.name;
		this.spawning = raw.spawning || null;
		this.energy = raw.store ? raw.store.energy || 0 : 0;
		this.energyCapacity = raw.energyCapacity;
	}
	get memory() {
		if (!Memory.spawns) Memory.spawns = {};
		return Memory.spawns[this.name];
	}
	spawnCreep(body, name, opts) {
		opts = opts || {};
		if (this.spawning) return C.ERR_BUSY;
		if (typeof name !== "string" || name.length > 100 || !name) return C.ERR_INVALID_ARGS;
		if (Game.creeps[name] || (Memory.creeps && Memory.creeps[name] && !Game.creeps[name] && false)) {
			if (Game.creeps[name]) return C.ERR_NAME_EXISTS;
		}
		if (!Array.isArray(body) || body.length === 0) return C.ERR_INVALID_ARGS;
		if (body.length > C.MAX_CREEP_SIZE) body = body.slice(0, C.MAX_CREEP_SIZE);
		let cost = 0;
		for (const p of body) {
			if (!C.BODYPART_COST[p]) return C.ERR_INVALID_ARGS;
			cost += C.BODYPART_COST[p];
		}
		const roomEnergy = this.room.energyAvailable;
		if (cost > roomEnergy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (opts.memory) {
			if (!Memory.creeps) Memory.creeps = {};
			Memory.creeps[name] = opts.memory;
		}
		intent(this.id, "spawnCreep", { body, name });
		return C.OK;
	}
	canCreateCreep() {
		return this.spawning ? C.ERR_BUSY : C.OK;
	}
	recycleCreep() { return C.OK; }
	renewCreep() { return C.OK; }
	toString() {
		return `[spawn ${this.name}]`;
	}
}
class StructureController extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.level = raw.level;
		this.progress = raw.progress;
		this.progressTotal = raw.progressTotal;
		this.ticksToDowngrade = raw.ticksToDowngrade;
		this.safeModeAvailable = raw.safeModeAvailable || 0;
		this.safeMode = raw.safeMode || null;
		this.safeModeCooldown = null;
		this.upgradeBlocked = raw.upgradeBlocked || 0;
		this.reservation = raw.reservation || null;
		this.sign = null;
		this.isPowerEnabled = false;
	}
	activateSafeMode() { return C.ERR_INVALID_ARGS; }
	unclaim() { return C.OK; }
	toString() {
		return `[controller ${this.pos.roomName}]`;
	}
}
class StructureExtension extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.energy = raw.store ? raw.store.energy || 0 : 0;
		this.energyCapacity = raw.energyCapacity;
	}
}
class StructureStorage extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.storeCapacity = raw.storeCapacity;
	}
}
class StructureContainer extends Structure {
	constructor(raw) {
		super(raw);
		this.storeCapacity = raw.storeCapacity;
		this.ticksToDecay = raw.ticksToDecay;
		this.owner = undefined;
		this.my = undefined;
	}
}
class StructureTerminal extends OwnedStructure { }
class StructureTower extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.energy = raw.store ? raw.store.energy || 0 : 0;
		this.energyCapacity = raw.energyCapacity;
	}
	attack() { return C.ERR_NOT_ENOUGH_ENERGY; }
	heal() { return C.ERR_NOT_ENOUGH_ENERGY; }
	repair() { return C.ERR_NOT_ENOUGH_ENERGY; }
}
class StructureLink extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.energy = raw.store ? raw.store.energy || 0 : 0;
		this.energyCapacity = raw.energyCapacity;
		this.cooldown = 0;
	}
	transferEnergy() { return C.ERR_INVALID_ARGS; }
}
class StructureLab extends OwnedStructure { }
class StructureNuker extends OwnedStructure { }
class StructureObserver extends OwnedStructure { observeRoom() { return C.ERR_INVALID_ARGS; } }
class StructurePowerSpawn extends OwnedStructure { processPower() { return C.ERR_NOT_ENOUGH_RESOURCES; } }
class StructureExtractor extends OwnedStructure { constructor(raw) { super(raw); this.cooldown = 0; } }
class StructureFactory extends OwnedStructure { constructor(raw) { super(raw); this.cooldown = 0; this.level = 0; } produce() { return C.ERR_INVALID_ARGS; } }
class StructureRoad extends Structure { constructor(raw) { super(raw); this.ticksToDecay = undefined; this.owner = undefined; this.my = undefined; } }
class StructureWall extends Structure { constructor(raw) { super(raw); this.owner = undefined; this.my = undefined; } }
class StructureRampart extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.isPublic = false;
		this.ticksToDecay = raw.ticksToDecay;
	}
	setPublic() { return C.OK; }
}

const STRUCTURE_CLASSES = {
	spawn: StructureSpawn,
	extension: StructureExtension,
	road: StructureRoad,
	constructedWall: StructureWall,
	rampart: StructureRampart,
	keeperLair: Structure,
	portal: Structure,
	controller: StructureController,
	link: StructureLink,
	storage: StructureStorage,
	tower: StructureTower,
	observer: StructureObserver,
	powerBank: Structure,
	powerSpawn: StructurePowerSpawn,
	extractor: StructureExtractor,
	lab: StructureLab,
	terminal: StructureTerminal,
	container: StructureContainer,
	nuker: StructureNuker,
	factory: StructureFactory,
	invaderCore: Structure,
};

class Flag extends RoomObject {
	constructor(name, color, secondaryColor, roomName, x, y) {
		super({ id: `flag_${name}`, x, y, roomName });
		this.name = name;
		this.color = color;
		this.secondaryColor = secondaryColor;
	}
	remove() {
		delete Game.flags[this.name];
		return C.OK;
	}
	setColor(color, secondaryColor) {
		this.color = color;
		this.secondaryColor = secondaryColor || color;
		return C.OK;
	}
	setPosition(x, y) {
		const pos = typeof x === "object" ? x : { x, y };
		this.pos = new RoomPosition(pos.x, pos.y, this.pos.roomName);
		return C.OK;
	}
}

// ═══════════════════════════ Terrain / Room ═══════════════════════════

class Terrain {
	constructor(roomName) {
		this.__roomName = roomName;
		this.__data = null;
	}
	get data() {
		if (!this.__data) this.__data = terrainOf(this.__roomName);
		return this.__data;
	}
	get(x, y) {
		return this.data[y * 50 + x];
	}
	getRawBuffer() {
		const copy = new Uint8Array(2500);
		copy.set(this.data);
		return copy;
	}
}

class Room {
	constructor(name, raw) {
		this.name = name;
		this.__raw = raw;
		this.mode = "world";
		this.controller = null;
		this.storage = null;
		this.terminal = null;
		this.visual = new RoomVisual(name);
		// 分类索引
		this.__objects = [];
		this.__byType = { creep: [], source: [], energy: [], constructionSite: [], tombstone: [], flag: [], powerCreep: [] };
		for (const st of Object.keys(STRUCTURE_CLASSES)) this.__byType[st] = [];
		this.__occupied = new Map(); // "x,y" → objects[]
		for (const rawObj of raw.objects) {
			const obj = wrapObject(this, rawObj);
			if (!obj) continue;
			this.__objects.push(obj);
			const key = `${rawObj.x},${rawObj.y}`;
			let cell = this.__occupied.get(key);
			if (!cell) this.__occupied.set(key, (cell = []));
			cell.push(obj);
			if (obj instanceof Structure) this.__byType[obj.structureType].push(obj);
			else if (this.__byType[rawObj.type]) this.__byType[rawObj.type].push(obj);
			if (obj instanceof StructureController && obj.my) this.controller = obj;
			if (obj instanceof StructureStorage && obj.my) this.storage = obj;
			if (obj instanceof StructureTerminal && obj.my) this.terminal = obj;
		}
		this.energyAvailable = raw.energyAvailable;
		this.energyCapacityAvailable = raw.energyCapacityAvailable;
	}
	get memory() {
		if (!Memory.rooms) Memory.rooms = {};
		return Memory.rooms[this.name];
	}
	getTerrain() {
		return new Terrain(this.name);
	}
	getPositionAt(x, y) {
		if (x < 0 || x > 49 || y < 0 || y > 49) return null;
		return new RoomPosition(x, y, this.name);
	}
	find(type, opts) {
		let results = collectFind(this.name, type);
		if (opts && typeof opts.filter === "function") {
			results = results.filter(opts.filter);
		}
		return results;
	}
	lookForAt(type, x, y) {
		if (typeof x === "object") { y = x.y; x = x.x; }
		if (x < 0 || x > 49 || y < 0 || y > 49) return [];
		const cell = this.__occupied.get(`${x},${y}`) || [];
		switch (type) {
			case C.LOOK_CREEPS: return cell.filter((o) => o instanceof Creep);
			case C.LOOK_ENERGY: return cell.filter((o) => o instanceof Resource);
			case C.LOOK_SOURCES: return cell.filter((o) => o instanceof Source);
			case C.LOOK_STRUCTURES: return cell.filter((o) => o instanceof Structure);
			case C.LOOK_CONSTRUCTION_SITES: return cell.filter((o) => o instanceof ConstructionSite);
			case C.LOOK_TOMBSTONES: return cell.filter((o) => o instanceof Tombstone);
			case C.LOOK_FLAGS: return cell.filter((o) => o instanceof Flag);
			case C.LOOK_TERRAIN: {
				const t = terrainOf(this.name)[y * 50 + x];
				return [t & C.TERRAIN_MASK_WALL ? "wall" : t & C.TERRAIN_MASK_SWAMP ? "swamp" : "plain"];
			}
			default: return [];
		}
	}
	lookAt(x, y) {
		if (typeof x === "object") { y = x.y; x = x.x; }
		const out = [];
		for (const type of ["creep", "energy", "source", "structure", "constructionSite", "tombstone", "terrain"]) {
			for (const v of this.lookForAt(type, x, y)) out.push({ type, [type]: v });
		}
		return out;
	}
	createConstructionSite(firstArg, secondArg, structureType, name) {
		let x, y;
		if (typeof secondArg === "number") [x, y] = [firstArg, secondArg];
		else if (typeof firstArg === "object") { x = firstArg.x; y = firstArg.y; }
		else if (typeof secondArg === "string") { x = firstArg; structureType = secondArg; }
		if (x === undefined || y === undefined || x < 0 || x > 49 || y < 0 || y > 49) return C.ERR_INVALID_ARGS;
		if (!C.CONSTRUCTION_COST[structureType]) return C.ERR_INVALID_ARGS;
		if (this.controller && this.controller.level > 0 && !this.controller.my) return C.ERR_NOT_OWNER;
		// RCL 数量上限
		const limits = this.__raw.structureLimits;
		if (limits && limits[structureType] !== undefined) {
			const existing = this.find(FIND_STRUCTURES, { filter: (s) => s.structureType === structureType })
				.length + this.find(FIND_CONSTRUCTION_SITES, { filter: (s) => s.structureType === structureType }).length;
			if (existing >= limits[structureType]) return C.ERR_RCL_NOT_ENOUGH;
		}
		if (Game.constructionSites && Object.keys(Game.constructionSites).length >= C.MAX_CONSTRUCTION_SITES) return C.ERR_FULL;
		// 占位检查
		const cell = this.__occupied.get(`${x},${y}`) || [];
		const blocking = cell.some((o) => {
			if (o instanceof Creep) return false;
			if (o instanceof ConstructionSite) return true;
			if (o instanceof Structure) {
				if (structureType === "rampart" || structureType === "road") return false;
				if (o.structureType === "rampart" || o.structureType === "road") return false;
				return true;
			}
			return false;
		});
		if (blocking) return C.ERR_INVALID_TARGET;
		if (structureType !== "road" && structureType !== "extractor" && structureType !== "container") {
			const t = terrainOf(this.name)[y * 50 + x];
			if (t & C.TERRAIN_MASK_WALL) return C.ERR_INVALID_TARGET;
		}
		intent("room", "createConstructionSite", { room: this.name, x, y, structureType, name });
		return C.OK;
	}
	createFlag(firstArg, secondArg, name, color, secondaryColor) {
		let x, y;
		if (typeof secondArg === "number") [x, y] = [firstArg, secondArg];
		else if (typeof firstArg === "object") { x = firstArg.x; y = firstArg.y; name = secondArg; }
		if (x === undefined || x < 0 || x > 49 || y < 0 || y > 49) return C.ERR_INVALID_ARGS;
		if (!color) color = C.COLOR_WHITE;
		if (!secondaryColor) secondaryColor = color;
		if (!name) {
			let cnt = 1;
			do name = `Flag${cnt++}`;
			while (Game.flags[name]);
		}
		if (Game.flags[name]) return C.ERR_NAME_EXISTS;
		Game.flags[name] = new Flag(name, color, secondaryColor, this.name, x, y);
		return name;
	}
	serializePath(path) {
		return serializePath(path);
	}
	deserializePath(path) {
		return deserializePath(path);
	}
	findPath(fromPos, toPos, opts) {
		return roomFindPathImpl(this.name, fromPos, toPos, opts);
	}
	findExitTo(room) {
		return Game.map.findExit(this.name, typeof room === "string" ? room : room.name);
	}
	getEventLog() {
		return [];
	}
	toString() {
		return `[room ${this.name}]`;
	}
}
Room.Terrain = Terrain;

function wrapObject(room, raw) {
	raw.roomName = room.name;
	switch (raw.type) {
		case "creep": return new Creep(raw);
		case "source": return new Source(raw);
		case "energy": return new Resource(raw);
		case "constructionSite": return new ConstructionSite(raw);
		case "tombstone": return new Tombstone(raw);
		default: {
			const cls = STRUCTURE_CLASSES[raw.type];
			if (!cls) return null;
			const obj = new cls({ ...raw, structureType: raw.type });
			return obj;
		}
	}
}

function collectFind(roomName, type, opts) {
	const room = Game.rooms[roomName];
	if (!room) return [];
	switch (type) {
		case C.FIND_CREEPS: return room.__byType.creep;
		case C.FIND_MY_CREEPS: return room.__byType.creep.filter((c) => c.my);
		case C.FIND_HOSTILE_CREEPS: return room.__byType.creep.filter((c) => !c.my);
		case C.FIND_SOURCES: return room.__byType.source;
		case C.FIND_SOURCES_ACTIVE: return room.__byType.source.filter((s) => s.energy > 0);
		case C.FIND_DROPPED_RESOURCES: return room.__byType.energy;
		case C.FIND_STRUCTURES: return room.__objects.filter((o) => o instanceof Structure);
		case C.FIND_MY_STRUCTURES: return room.__objects.filter((o) => o instanceof OwnedStructure && o.my);
		case C.FIND_CONSTRUCTION_SITES:
		case C.FIND_MY_CONSTRUCTION_SITES: return room.__byType.constructionSite.filter((s) => s.my);
		case C.FIND_TOMBSTONES: return room.__byType.tombstone;
		case C.FIND_FLAGS: return Object.values(Game.flags).filter((f) => f.pos.roomName === roomName);
		case C.FIND_MY_SPAWNS: return room.__byType.spawn.filter((s) => s.my);
		case C.FIND_HOSTILE_SPAWNS: return room.__byType.spawn.filter((s) => !s.my);
		case C.FIND_EXIT_TOP: return exitPositions(roomName, 1);
		case C.FIND_EXIT_RIGHT: return exitPositions(roomName, 3);
		case C.FIND_EXIT_BOTTOM: return exitPositions(roomName, 5);
		case C.FIND_EXIT_LEFT: return exitPositions(roomName, 7);
		case C.FIND_EXIT: {
			const exits = [];
			for (const dir of [1, 3, 5, 7]) exits.push(...exitPositions(roomName, dir));
			return exits;
		}
		default: return [];
	}
}

function exitPositions(roomName, dir) {
	const terrain = terrainOf(roomName);
	const out = [];
	for (let i = 0; i < 50; i++) {
		const [x, y] = dir === 1 ? [i, 0] : dir === 3 ? [49, i] : dir === 5 ? [i, 49] : [0, i];
		if (!(terrain[y * 50 + x] & C.TERRAIN_MASK_WALL)) out.push(new RoomPosition(x, y, roomName));
	}
	return out;
}

function findClosestImpl(pos, type, opts, mode) {
	const objects = typeof type === "number" ? collectFind(pos.roomName, type, opts) : type;
	if (!objects || objects.length === 0) return null;
	const onSquare = objects.find((o) => (o.pos || o).isEqualTo(pos));
	if (onSquare) return onSquare;
	if (mode === "range") {
		let best = null, bestRange = Infinity;
		for (const o of objects) {
			const r = pos.getRangeTo(o.pos || o);
			if (r < bestRange) { bestRange = r; best = o; }
		}
		return best;
	}
	// path 模式: 多目标 PathFinder
	const goals = objects.map((o) => ({ pos: o.pos || o, range: 1 }));
	const searchOpts = buildMoveSearchOpts(pos.roomName, opts);
	const result = pathFinderSearch(pos, goals, searchOpts);
	if (result.incomplete || result.path.length === 0) return null;
	const last = result.path[result.path.length - 1];
	return objects.find((o) => (o.pos || o).isNearTo(last) || (o.pos || o).isEqualTo(last)) || null;
}

/** moveTo/findPath 的搜索参数 (官方 _findPath2: 默认考虑道路, plain 2 / swamp 10) */
function buildMoveSearchOpts(roomName, opts) {
	opts = opts || {};
	const ignoreCreeps = opts.ignoreCreeps !== undefined ? opts.ignoreCreeps : true;
	return {
		roomCallback: (name) => buildPathMatrix(name, { ignoreCreeps, ignoreDestructibleStructures: opts.ignoreDestructibleStructures, ignoreRoads: opts.ignoreRoads }),
		plainCost: opts.plainCost || (opts.ignoreRoads ? undefined : 2),
		swampCost: opts.swampCost || (opts.ignoreRoads ? undefined : 10),
		maxOps: opts.maxOps,
		maxRooms: opts.maxRooms,
		heuristicWeight: opts.heuristicWeight,
		range: opts.range,
	};
}

/** 官方 makePathfindingGrid2: 障碍 255, 路 1 (沼泽交给 swampCost) */
const matrixCache = new Map();
function buildPathMatrix(roomName, opts) {
	if (!Game.rooms[roomName]) return false;
	const room = Game.rooms[roomName];
	const cacheKey = `${roomName}|${!!opts.ignoreCreeps}|${!!opts.ignoreDestructibleStructures}|${!!opts.ignoreRoads}|${Game.time}`;
	if (matrixCache.has(cacheKey)) return matrixCache.get(cacheKey);
	const costs = new CostMatrix();
	for (const o of room.__objects) {
		if (o instanceof Creep) {
			if (!opts.ignoreCreeps) costs.set(o.pos.x, o.pos.y, 0xff);
			continue;
		}
		if (o instanceof ConstructionSite) {
			if (o.my && C.OBSTACLE_OBJECT_TYPES.includes(o.structureType)) costs.set(o.pos.x, o.pos.y, 0xff);
			continue;
		}
		if (o instanceof Structure) {
			const obstacle = C.OBSTACLE_OBJECT_TYPES.includes(o.structureType) ||
				(o.structureType === "rampart" && !o.isPublic && !o.my);
			const destructible = ["constructedWall", "spawn", "extension", "link", "storage", "observer", "tower", "powerBank", "powerSpawn", "lab", "terminal"];
			if (obstacle && !(opts.ignoreDestructibleStructures && destructible.includes(o.structureType))) {
				costs.set(o.pos.x, o.pos.y, 0xff);
			}
			if (o.structureType === "road" && !opts.ignoreRoads && costs.get(o.pos.x, o.pos.y) < 0xff) {
				costs.set(o.pos.x, o.pos.y, 1);
			}
		}
	}
	matrixCache.set(cacheKey, costs);
	if (matrixCache.size > 200) matrixCache.clear();
	return costs;
}

// ═══════════════════════════ 序列化路径 ═══════════════════════════

function serializePath(path) {
	return path.map((i) => `${i.x}.${i.y}.${i.direction}`).join("_");
}
function deserializePath(str) {
	if (!str) return [];
	return str.split("_").map((chunk) => {
		const [x, y, direction] = chunk.split(".");
		const dx = OFFSETS[+direction][0], dy = OFFSETS[+direction][1];
		return { x: +x, y: +y, dx, dy, direction: +direction };
	});
}

/** Room.findPath / findPathTo 的共享实现 (官方 _findPath2 语义) */
function roomFindPathImpl(roomName, fromPos, toPos, opts) {
	if (fromPos.isEqualTo(toPos)) return [];
	opts = opts || {};
	if (opts.maxOps === undefined && (opts.maxRooms === undefined || opts.maxRooms > 1) && fromPos.roomName !== toPos.roomName) {
		opts.maxOps = 20000;
	}
	const moveOpts = buildMoveSearchOpts(roomName, opts);
	const range = Math.max(1, opts.range || 0);
	const ret = pathFinderSearch(fromPos, [{ pos: toPos, range }], moveOpts);
	let path = ret.path;
	if (!opts.range && path.length && path[path.length - 1].isNearTo(toPos) && !path[path.length - 1].isEqualTo(toPos)) {
		path.push(toPos);
	} else if (!opts.range && !path.length && fromPos.isNearTo(toPos)) {
		path = [toPos];
	}
	const resultPath = [];
	let curX = fromPos.x, curY = fromPos.y;
	for (const pos of path) {
		if (pos.roomName !== roomName) break;
		if (Math.abs(pos.x - curX) > 1 || Math.abs(pos.y - curY) > 1) break;
		resultPath.push({
			x: pos.x, y: pos.y,
			dx: pos.x - curX, dy: pos.y - curY,
			direction: getDirection(pos.x - curX, pos.y - curY),
		});
		curX = pos.x;
		curY = pos.y;
	}
	if (opts.serialize) return serializePath(resultPath);
	return resultPath;
}

// ═══════════════════════════ PathFinder ═══════════════════════════

class CostMatrix {
	constructor(data) {
		this.__bits = data ? Uint8Array.from(data) : new Uint8Array(2500);
	}
	set(x, y, value) {
		if (x < 0 || x > 49 || y < 0 || y > 49) return this;
		this.__bits[y * 50 + x] = value;
		return this;
	}
	get(x, y) {
		if (x < 0 || x > 49 || y < 0 || y > 49) return 0xff;
		return this.__bits[y * 50 + x];
	}
	clone() {
		return new CostMatrix(this.__bits);
	}
	serialize() {
		return Array.from(this.__bits).join(",");
	}
	static deserialize(str) {
		const parts = str.split(",");
		const bits = new Uint8Array(2500);
		for (let i = 0; i < 2500; i++) bits[i] = +parts[i] || 0;
		return new CostMatrix(bits);
	}
}

/** 二叉最小堆 */
class MinHeap {
	constructor() {
		this.a = [];
	}
	get size() { return this.a.length; }
	push(node) {
		const a = this.a;
		a.push(node);
		let i = a.length - 1;
		while (i > 0) {
			const p = (i - 1) >> 1;
			if (a[p].f <= a[i].f) break;
			[a[p], a[i]] = [a[i], a[p]];
			i = p;
		}
	}
	pop() {
		const a = this.a;
		const top = a[0];
		const last = a.pop();
		if (a.length) {
			a[0] = last;
			let i = 0;
			for (;;) {
				const l = i * 2 + 1, r = l + 1;
				let m = i;
				if (l < a.length && a[l].f < a[m].f) m = l;
				if (r < a.length && a[r].f < a[m].f) m = r;
				if (m === i) break;
				[a[m], a[i]] = [a[i], a[m]];
				i = m;
			}
		}
		return top;
	}
}

const OP_LIMIT_ROOMS = 16, OP_LIMIT_OPS = 2000;

/**
 * PathFinder.search: 多房间 A* (typed array + 二叉堆)。
 * 房间邻接按世界房名推导; roomCallback 返回 false 排除房间;
 * CostMatrix 0 = 地形默认 (plain/swampCost), 255 = 不可通行。
 * 近似点: 跨房边界按"出口格 + 对侧入口格"两步计价, 官方为同节点;
 * 启发式用世界坐标 octile × heuristicWeight。
 */
function pathFinderSearch(origin, goal, opts) {
	opts = opts || {};
	const goals = Array.isArray(goal) ? goal : [goal];
	const plainCost = opts.plainCost || 1;
	const swampCost = opts.swampCost || 5;
	const maxRooms = Math.max(1, opts.maxRooms || OP_LIMIT_ROOMS);
	const maxOps = opts.maxOps || OP_LIMIT_OPS;
	const maxCost = opts.maxCost || Infinity;
	const hw = opts.heuristicWeight === undefined ? 1.2 : opts.heuristicWeight;

	// 房间注册表
	const roomIndex = new Map(); // name → idx
	const roomData = []; // {name, terrain, matrix, exits}
	function registerRoom(name) {
		if (roomIndex.has(name)) return roomIndex.get(name);
		if (roomIndex.size >= maxRooms) return -1;
		if (!__W.roomNames.includes(name)) return -1;
		let matrix = null;
		if (opts.roomCallback) {
			const ret = opts.roomCallback(name);
			if (ret === false) { roomIndex.set(name, -1); return -1; }
			if (ret instanceof CostMatrix) matrix = ret.__bits;
		}
		const idx = roomData.length;
		const [rx, ry] = roomNameToXY(name);
		roomData.push({ name, terrain: terrainOf(name), matrix, rx, ry });
		roomIndex.set(name, idx);
		return idx;
	}
	const originIdx = registerRoom(origin.roomName);
	if (originIdx < 0) return { path: [], ops: 0, cost: 0, incomplete: true };
	const oRoom = roomData[originIdx];

	function tileCost(room, x, y) {
		const m = room.matrix ? room.matrix[y * 50 + x] : 0;
		if (m === 0xff) return 0xffff;
		if (m > 0) return m;
		const t = room.terrain[y * 50 + x];
		if (t & C.TERRAIN_MASK_WALL) return 0xffff;
		return t & C.TERRAIN_MASK_SWAMP ? swampCost : plainCost;
	}

	// 目标世界坐标 (启发式)
	const goalAnchors = goals.map((g) => {
		const [gx, gy] = roomNameToXY(g.pos.roomName);
		return { wx: gx * 50 + g.pos.x, wy: gy * 50 + g.pos.y, range: g.range || 0, goal: g };
	});
	function heuristic(roomIdx, x, y) {
		const room = roomData[roomIdx];
		const wx = room.rx * 50 + x, wy = room.ry * 50 + y;
		let best = Infinity;
		for (const a of goalAnchors) {
			const dx = Math.max(Math.abs(wx - a.wx) - a.range, 0);
			const dy = Math.max(Math.abs(wy - a.wy) - a.range, 0);
			const h = dx > dy ? dx + 0.41421356 * dy : dy + 0.41421356 * dx;
			if (h < best) best = h;
		}
		return best * hw;
	}
	function inGoal(roomIdx, x, y) {
		const room = roomData[roomIdx];
		for (const a of goalAnchors) {
			if (a.goal.pos.roomName !== room.name) continue;
			const r = Math.max(Math.abs(x - a.goal.pos.x), Math.abs(y - a.goal.pos.y));
			if (r <= a.range) return true;
		}
		return false;
	}

	// 状态数组: 每房间 2500 格
	const gCost = [], closed = [], parent = [];
	const ensureRoom = (idx) => {
		if (!gCost[idx]) { gCost[idx] = new Float64Array(2500).fill(Infinity); closed[idx] = new Uint8Array(2500); parent[idx] = new Int32Array(2500 * 2).fill(-1); }
	};

	const startNode = { f: heuristic(originIdx, origin.x, origin.y), g: 0, room: originIdx, x: origin.x, y: origin.y };
	const heap = new MinHeap();
	heap.push(startNode);
	ensureRoom(originIdx);
	gCost[originIdx][origin.y * 50 + origin.x] = 0;
	let ops = 0;
	let found = null;
	if (inGoal(originIdx, origin.x, origin.y)) found = { room: originIdx, x: origin.x, y: origin.y };

	while (!found && heap.size && ops < maxOps) {
		const node = heap.pop();
		ops++;
		const { room: ridx, x, y } = node;
		const ci = y * 50 + x;
		if (closed[ridx][ci]) continue;
		closed[ridx][ci] = 1;
		if (node.g > gCost[ridx][ci]) continue;
		if (inGoal(ridx, x, y)) { found = { room: ridx, x, y }; break; }

		const room = roomData[ridx];
		for (let dir = 1; dir <= 8; dir++) {
			const [dx, dy] = OFFSETS[dir];
			const nx = x + dx, ny = y + dy;
			if (nx < 0 || nx > 49 || ny < 0 || ny > 49) continue;
			const step = tileCost(room, nx, ny);
			if (step >= 0xffff) continue;
			const ng = node.g + step;
			if (ng > maxCost) continue;
			ensureRoom(ridx);
			const ni = ny * 50 + nx;
			if (ng < gCost[ridx][ni]) {
				gCost[ridx][ni] = ng;
				parent[ridx][ni * 2] = ci;
				parent[ridx][ni * 2 + 1] = ridx;
				heap.push({ f: ng + heuristic(ridx, nx, ny), g: ng, room: ridx, x: nx, y: ny });
			}
		}
		// 跨房: 从边界格跳到相邻房间对侧格
		if (x === 0 || x === 49 || y === 0 || y === 49) {
			const crossings = [];
			if (x === 0) crossings.push([room.rx - 1, room.ry, 49, y]);
			if (x === 49) crossings.push([room.rx + 1, room.ry, 0, y]);
			if (y === 0) crossings.push([room.rx, room.ry - 1, x, 49]);
			if (y === 49) crossings.push([room.rx, room.ry + 1, x, 0]);
			for (const [nrx, nry, nx, ny] of crossings) {
				const nName = xyToRoomName(nrx, nry);
				const nIdx = registerRoom(nName);
				if (nIdx < 0) continue;
				const nRoom = roomData[nIdx];
				const step = tileCost(nRoom, nx, ny);
				if (step >= 0xffff) continue;
				const ng = node.g + step;
				ensureRoom(nIdx);
				const ni = ny * 50 + nx;
				if (ng < gCost[nIdx][ni]) {
					gCost[nIdx][ni] = ng;
					parent[nIdx][ni * 2] = ci;
					parent[nIdx][ni * 2 + 1] = ridx;
					heap.push({ f: ng + heuristic(nIdx, nx, ny), g: ng, room: nIdx, x: nx, y: ny });
				}
			}
		}
	}

	const path = [];
	let cost = 0;
	if (found) {
		let { room: ridx, x, y } = found;
		cost = gCost[ridx][y * 50 + x];
		const originRoom = roomData[originIdx].name;
		for (;;) {
			path.push(new RoomPosition(x, y, roomData[ridx].name));
			if (ridx === originIdx && x === origin.x && y === origin.y) break;
			const ti = y * 50 + x;
			const pRoom = parent[ridx][ti * 2 + 1];
			const pTile = parent[ridx][ti * 2];
			if (pRoom < 0 || pTile < 0) break;
			ridx = pRoom;
			x = pTile % 50;
			y = Math.floor(pTile / 50);
		}
		path.reverse();
		// 起点移除; 若回溯链未精确落在起点 (不应发生), 以起点最近一跳截断
		if (path.length && path[0].isEqualTo(origin)) path.shift();
		const oi = path.findIndex((p) => p.isEqualTo(origin));
		if (oi > 0) path.splice(0, oi);
	}
	return { path, ops, cost, incomplete: !found };
}

const PathFinder = {
	CostMatrix,
	search: pathFinderSearch,
	use() {},
};

// ═══════════════════════════ RoomVisual (no-op) ═══════════════════════════

class RoomVisual {
	constructor(roomName) {
		this.roomName = roomName;
	}
	line() { return this; }
	circle() { return this; }
	rect() { return this; }
	poly() { return this; }
	text() { return this; }
	clear() { return this; }
	getSize() { return 0; }
	export() { return ""; }
	import() { return this; }
}

// ═══════════════════════════ Game 组装 ═══════════════════════════

// var 声明 (而非 let): 挂到 globalThis 属性, 避免全局词法绑定遮蔽 bot 侧同名全局
var byId = new Map();
var Game = null;
var Memory = null;

function buildGame() {
	byId = new Map();
	Game = {
		time: __W.time,
		rooms: {},
		creeps: {},
		spawns: {},
		structures: {},
		constructionSites: {},
		flags: {},
		powerCreeps: {},
		resources: {},
		cpu: makeCpu(__W.cpu),
		gcl: __W.gcl,
		gpl: { level: 0, progress: 0, progressTotal: 0 },
		shard: { name: "shard0", type: "world", ptr: false },
		market: {
			credits: 0, incomingTransactions: [], outgoingTransactions: [], orders: {},
			calcTransactionCost() { return Infinity; }, createOrder() { return C.ERR_INVALID_ARGS; },
			deal() { return C.ERR_INVALID_ARGS; }, cancelOrder() { return C.OK; }, changeOrderPrice() { return C.OK; },
			extendOrder() { return C.OK; }, getAllOrders() { return []; }, getHistory() { return []; }, getOrderById() { return null; },
		},
		map: makeMap(),
		getObjectById(id) {
			return byId.get(id) || null;
		},
		notify() {},
	};
	for (const [name, rawRoom] of Object.entries(__W.rooms)) {
		const room = new Room(name, rawRoom);
		Game.rooms[name] = room;
		for (const obj of room.__objects) {
			if (!obj) continue;
			byId.set(obj.id, obj);
			if (obj instanceof Creep && obj.my) Game.creeps[obj.name] = obj;
			if (obj instanceof StructureSpawn && obj.my) Game.spawns[obj.name] = obj;
			if (obj instanceof Structure && obj.id) Game.structures[obj.id] = obj;
			if (obj instanceof ConstructionSite && obj.my) Game.constructionSites[obj.id] = obj;
		}
	}
	// RCL 结构上限表 (createConstructionSite 校验用)
	for (const room of Object.values(Game.rooms)) {
		if (room.controller) {
			room.__raw.structureLimits = structureLimitsFor(room.controller.level);
		}
	}
}

function structureLimitsFor(rcl) {
	return {
		spawn: [0, 1, 1, 1, 1, 1, 1, 2, 3][Math.min(rcl, 8)],
		extension: [0, 0, 5, 10, 20, 30, 40, 50, 60][Math.min(rcl, 8)],
		road: 2500,
		constructedWall: [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500][Math.min(rcl, 8)],
		rampart: [0, 0, 2500, 2500, 2500, 2500, 2500, 2500, 2500][Math.min(rcl, 8)],
		link: [0, 0, 0, 0, 0, 2, 3, 4, 6][Math.min(rcl, 8)],
		storage: [0, 0, 0, 1, 1, 1, 1, 1, 1][Math.min(rcl, 8)],
		tower: [0, 0, 0, 1, 1, 2, 2, 3, 6][Math.min(rcl, 8)],
		observer: [0, 0, 0, 0, 0, 0, 0, 0, 1][Math.min(rcl, 8)],
		powerSpawn: [0, 0, 0, 0, 0, 0, 0, 1, 1][Math.min(rcl, 8)],
		extractor: [0, 0, 0, 0, 0, 1, 1, 1, 1][Math.min(rcl, 8)],
		lab: [0, 0, 0, 0, 0, 3, 6, 10, 10][Math.min(rcl, 8)],
		terminal: [0, 0, 0, 0, 0, 1, 1, 1, 1][Math.min(rcl, 8)],
		container: 5,
		nuker: [0, 0, 0, 0, 0, 0, 0, 0, 1][Math.min(rcl, 8)],
		factory: [0, 0, 0, 0, 0, 0, 0, 1, 1][Math.min(rcl, 8)],
	};
}

function makeCpu(raw) {
	const start = nowMs();
	return {
		limit: raw.limit,
		tickLimit: raw.tickLimit,
		bucket: raw.bucket,
		shardLimits: { shard0: raw.limit },
		unlocked: false,
		unlockedTime: undefined,
		getUsed() { return nowMs() - start; },
		getHeapStatistics() {
			return { total_heap_size: 0, used_heap_size: 0, heap_size_limit: 0, externally_allocated_size: 0 };
		},
		halt() {},
		setShardLimits() { return C.ERR_INVALID_ARGS; },
		unlock() { return C.ERR_FULL; },
		generatePixel() { return C.ERR_NOT_ENOUGH_RESOURCES; },
	};
}

function makeMap() {
	return {
		getWorldSize() { return __W.roomNames.length > 1 ? 200 : 50; },
		describeExits(roomName) {
			if (!__W.roomNames.includes(roomName)) return null;
			const [rx, ry] = roomNameToXY(roomName);
			const exits = {};
			const set = (dir, name) => { if (__W.roomNames.includes(name)) exits[dir] = name; };
			set("1", xyToRoomName(rx, ry - 1));
			set("3", xyToRoomName(rx + 1, ry));
			set("5", xyToRoomName(rx, ry + 1));
			set("7", xyToRoomName(rx - 1, ry));
			return exits;
		},
		findExit(fromRoom, toRoom, opts) {
			const route = this.findRoute(fromRoom, toRoom, opts);
			if (!Array.isArray(route)) return route;
			if (route.length === 0) return C.ERR_INVALID_ARGS;
			return route[0].exit;
		},
		findRoute(fromRoom, toRoom, opts) {
			const from = typeof fromRoom === "string" ? fromRoom : fromRoom.name;
			const to = typeof toRoom === "string" ? toRoom : toRoom.name;
			if (!__W.roomNames.includes(from) || !__W.roomNames.includes(to)) return C.ERR_NO_PATH;
			const cb = opts && opts.routeCallback;
			// Dijkstra
			const dist = new Map([[from, 0]]);
			const prev = new Map();
			const visited = new Set();
			for (;;) {
				let cur = null, curD = Infinity;
				for (const [name, d] of dist) {
					if (!visited.has(name) && d < curD) { cur = name; curD = d; }
				}
				if (cur === null || cur === to) break;
				visited.add(cur);
				const [rx, ry] = roomNameToXY(cur);
				for (const [dir, name] of Object.entries(this.describeExits(cur) || {})) {
					let cost = 1;
					if (cb) cost = cb(name, cur);
					if (cost === Infinity || cost === false || cost === undefined) continue;
					const nd = curD + cost;
					if (nd < (dist.get(name) ?? Infinity)) {
						dist.set(name, nd);
						prev.set(name, { from: cur, exit: +dir });
					}
				}
				const _ = [rx, ry]; void _;
			}
			if (!dist.has(to)) return C.ERR_NO_PATH;
			const route = [];
			let cur = to;
			while (cur !== from) {
				const p = prev.get(cur);
				if (!p) break;
				route.unshift({ exit: p.exit, room: cur });
				cur = p.from;
			}
			return route;
		},
		getRoomLinearDistance(roomName1, roomName2) {
			const [x1, y1] = roomNameToXY(roomName1);
			const [x2, y2] = roomNameToXY(roomName2);
			return Math.max(Math.abs(x1 - x2), Math.abs(y1 - y2));
		},
		getRoomTerrain(roomName) {
			return new Terrain(roomName);
		},
		getTerrainAt(x, y, roomName) {
			return new Terrain(roomName).get(x, y);
		},
		isRoomAvailable(roomName) {
			return __W.roomNames.includes(roomName);
		},
		getRoomStatus(roomName) {
			if (!__W.roomNames.includes(roomName)) return null;
			return { status: "normal", timestamp: 0 };
		},
		visual: new RoomVisual("map"),
	};
}

// ═══════════════════════════ 计时 / 随机 / 控制台 ═══════════════════════════

const __ops = globalThis.Deno && globalThis.Deno.core ? globalThis.Deno.core.ops : null;
const runtimeStart = __ops && __ops.op_sim_now ? __ops.op_sim_now() : 0;
function nowMs() {
	return __ops && __ops.op_sim_now ? __ops.op_sim_now() - runtimeStart : Date.now() - runtimeStart;
}

/** 可复现实验: Math.random 换 mulberry32 */
function seedRandom(seed) {
	let a = seed >>> 0;
	Math.random = function () {
		a |= 0; a = (a + 0x6D2B79F5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

const console = {
	log: (...args) => __consoleLines.push(args.map(fmt).join(" ")),
	info: (...args) => __consoleLines.push(args.map(fmt).join(" ")),
	warn: (...args) => __consoleLines.push(args.map(fmt).join(" ")),
	error: (...args) => __consoleLines.push(args.map(fmt).join(" ")),
};
globalThis.console = console;
function fmt(v) {
	if (typeof v === "string") return v;
	try { return JSON.stringify(v); } catch { return String(v); }
}

// ═══════════════════════════ RawMemory / 全局暴露 ═══════════════════════════

let rawMemoryData = "";
const RawMemory = {
	get() { return rawMemoryData; },
	set(value) { rawMemoryData = value; return C.OK; },
	segments: {},
	foreignSegment: null,
	interShardSegment: null,
	setActiveSegments() {},
	setActiveForeignSegment() {},
	setDefaultPublicSegment() {},
	setPublicSegments() {},
};
const InterShardMemory = {
	getLocal() { return ""; },
	setLocal() {},
	getRemote() { return ""; },
};

/** 全局类导出: 真实实现的直接挂载, 未实现的挂占位基类 */
const GLOBAL_CLASSES = {
	RoomPosition, Room, RoomObject, RoomVisual, Creep, Source, Resource,
	ConstructionSite, Tombstone, Flag, Structure, OwnedStructure, Store,
	StructureSpawn, StructureController, StructureExtension, StructureStorage,
	StructureContainer, StructureTerminal, StructureTower, StructureLink, StructureLab,
	StructureNuker, StructureObserver, StructurePowerSpawn, StructureExtractor,
	StructureFactory, StructureRoad, StructureWall, StructureRampart,
	Ruin: RoomObject, Mineral: RoomObject, Deposit: RoomObject, Nuke: RoomObject, PowerCreep: RoomObject,
};
for (const [name, cls] of Object.entries(GLOBAL_CLASSES)) {
	globalThis[name] = cls;
}
// PathFinder 命名空间 + CostMatrix 全局
globalThis.PathFinder = PathFinder;
globalThis.CostMatrix = CostMatrix;
globalThis.RawMemory = RawMemory;
globalThis.InterShardMemory = InterShardMemory;

// ═══════════════════════════ 主循环接口 ═══════════════════════════

/** bot 的 CJS module (bundle 执行后挂上); 模块级代码可能读 Memory/Game, 先给最小桩 */
let __botModule = null;

function installBootstrapGlobals() {
	if (!globalThis.Memory) globalThis.Memory = {};
	if (!globalThis.Game) {
		globalThis.Game = {
			time: 0,
			rooms: {}, creeps: {}, spawns: {}, structures: {}, constructionSites: {},
			flags: {}, powerCreeps: {}, resources: {},
			cpu: { limit: 100, tickLimit: 500, bucket: 10000, getUsed: () => 0, shardLimits: {}, unlocked: false, getHeapStatistics: () => ({}), halt() {}, setShardLimits() { return -10; }, unlock() { return -8; }, generatePixel() { return -6; } },
			gcl: { level: 1, progress: 0, progressTotal: 1000000 },
			gpl: { level: 0, progress: 0, progressTotal: 0 },
			shard: { name: "shard0", type: "world", ptr: false },
			market: { credits: 0, incomingTransactions: [], outgoingTransactions: [], orders: {}, calcTransactionCost: () => Infinity, getAllOrders: () => [], getHistory: () => [], getOrderById: () => null, createOrder: () => -10, deal: () => -10, cancelOrder: () => 0, changeOrderPrice: () => 0, extendOrder: () => 0 },
			map: null,
			getObjectById: () => null,
			notify() {},
		};
	}
}

function __bootstrapBundle(bundleCode) {
	installBootstrapGlobals();
	const module = { exports: {} };
	const exports = module.exports;
	globalThis.__require = (name) => ({});
	try {
		const fn = new Function("module", "exports", "require", `"use strict";\n${bundleCode}\n`);
		fn(module, exports, globalThis.__require);
	} catch (e) {
		return `bundle 加载失败: ${e && e.stack ? e.stack : e}`;
	}
	__botModule = module;
	return null;
}

/** 每 tick 调用: 输入经 op 获取, 结果经 op 提交 (不依赖全局字符串注入) */
function __runTick() {
	const errors = [];
	let cpuUsed = 0;
	__intents = [];
	__consoleLines = [];
	try {
		__W = JSON.parse(__ops.op_sim_get_snapshot());
		buildGame();
		rawMemoryData = __ops.op_sim_get_memory();
		try {
			Memory = rawMemoryData ? JSON.parse(rawMemoryData) : {};
		} catch {
			Memory = {};
		}
		globalThis.Game = Game;
		globalThis.Memory = Memory;

		const loop = __botModule && __botModule.exports && __botModule.exports.loop;
		if (typeof loop === "function") {
			const t0 = nowMs();
			try {
				loop();
			} catch (e) {
				errors.push(e && e.stack ? String(e.stack) : String(e));
			}
			cpuUsed = nowMs() - t0;
		} else {
			errors.push("bot bundle 未导出 loop 函数");
		}
	} catch (e) {
		errors.push(e && e.stack ? String(e.stack) : String(e));
	}
	let memoryOut = "";
	try {
		memoryOut = JSON.stringify(globalThis.Memory || {});
	} catch {
		memoryOut = "{}";
	}
	const result = JSON.stringify({
		intents: __intents,
		memory: memoryOut,
		cpuUsed,
		errors,
		console: __consoleLines.slice(0, 50),
	});
	__ops.op_sim_result(result);
	return result;
}

function __seedRandom(seed) {
	seedRandom(seed >>> 0);
}
