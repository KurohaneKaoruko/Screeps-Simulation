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
	ERR_NOT_ENOUGH_EXTENSIONS: -6,
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
	SPAWN_RENEW_RATIO: 1.2,
	MAX_CREEP_SIZE: 50,
	MAX_CONSTRUCTION_SITES: 100,

	BODYPART_COST: { move: 50, work: 100, carry: 50, attack: 80, ranged_attack: 150, tough: 10, heal: 250, claim: 600 },

	BODYPARTS_ALL: ["move", "work", "carry", "attack", "ranged_attack", "tough", "heal", "claim"],
	// 官方 RESOURCES_ALL (common/lib/constants.js)。此前是拼装出的错误列表 (含 HH/OO)。
	RESOURCES_ALL: [
		"energy", "power",
		"H", "O", "U", "L", "K", "Z", "X", "G",
		"OH", "ZK", "UL",
		"UH", "UO", "KH", "KO", "LH", "LO", "ZH", "ZO", "GH", "GO",
		"UH2O", "UHO2", "KH2O", "KHO2", "LH2O", "LHO2", "ZH2O", "ZHO2", "GH2O", "GHO2",
		"XUH2O", "XUHO2", "XKH2O", "XKHO2", "XLH2O", "XLHO2", "XZH2O", "XZHO2", "XGH2O", "XGHO2",
		"ops", "silicon", "metal", "biomass", "mist",
		"utrium_bar", "lemergium_bar", "zynthium_bar", "keanium_bar", "ghodium_melt",
		"utrium_oxide", "lemergium_oxide", "zynthium_oxide", "keanium_oxide", "ghodium_oxide",
		"utrium_acid", "lemergium_acid", "zynthium_acid", "keanium_acid", "ghodium_acid",
		"utrium_alkalide", "lemergium_alkalide", "zynthium_alkalide", "keanium_alkalide", "ghodium_alkalide",
		"catalyzed_utrium_acid", "catalyzed_lemergium_acid", "catalyzed_zynthium_acid", "catalyzed_keanium_acid", "catalyzed_ghodium_acid",
		"catalyzed_utrium_alkalide", "catalyzed_lemergium_alkalide", "catalyzed_zynthium_alkalide", "catalyzed_keanium_alkalide", "catalyzed_ghodium_alkalide",
	],
	INTERSHARD_RESOURCES: ["token"],

	RESOURCE_ENERGY: "energy",
	RESOURCE_POWER: "power",
	RESOURCE_OPS: "ops",
	// ── 基础矿物 ──
	RESOURCE_HYDROGEN: "H",
	RESOURCE_OXYGEN: "O",
	RESOURCE_UTRIUM: "U",
	RESOURCE_LEMERGIUM: "L",
	RESOURCE_KEANIUM: "K",
	RESOURCE_ZYNTHIUM: "Z",
	RESOURCE_CATALYST: "X",
	RESOURCE_GHODIUM: "G",
	// ── 化合物 ──
	RESOURCE_HYDROXIDE: "OH",
	RESOURCE_ZYNTHIUM_KEANIUM: "ZK",
	RESOURCE_UTRIUM_LEMERGIUM: "UL",
	RESOURCE_UTRIUM_HYDRIDE: "UH",
	RESOURCE_UTRIUM_OXIDE: "UO",
	RESOURCE_KEANIUM_HYDRIDE: "KH",
	RESOURCE_KEANIUM_OXIDE: "KO",
	RESOURCE_LEMERGIUM_HYDRIDE: "LH",
	RESOURCE_LEMERGIUM_OXIDE: "LO",
	RESOURCE_ZYNTHIUM_HYDRIDE: "ZH",
	RESOURCE_ZYNTHIUM_OXIDE: "ZO",
	RESOURCE_GHODIUM_HYDRIDE: "GH",
	RESOURCE_GHODIUM_OXIDE: "GO",
	// ── Boost ──
	RESOURCE_UTRIUM_ACID: "UH2O",
	RESOURCE_UTRIUM_ALKALIDE: "UHO2",
	RESOURCE_KEANIUM_ACID: "KH2O",
	RESOURCE_KEANIUM_ALKALIDE: "KHO2",
	RESOURCE_LEMERGIUM_ACID: "LH2O",
	RESOURCE_LEMERGIUM_ALKALIDE: "LHO2",
	RESOURCE_ZYNTHIUM_ACID: "ZH2O",
	RESOURCE_ZYNTHIUM_ALKALIDE: "ZHO2",
	RESOURCE_GHODIUM_ACID: "GH2O",
	RESOURCE_GHODIUM_ALKALIDE: "GHO2",
	RESOURCE_CATALYZED_UTRIUM_ACID: "XUH2O",
	RESOURCE_CATALYZED_UTRIUM_ALKALIDE: "XUHO2",
	RESOURCE_CATALYZED_KEANIUM_ACID: "XKH2O",
	RESOURCE_CATALYZED_KEANIUM_ALKALIDE: "XKHO2",
	RESOURCE_CATALYZED_LEMERGIUM_ACID: "XLH2O",
	RESOURCE_CATALYZED_LEMERGIUM_ALKALIDE: "XLHO2",
	RESOURCE_CATALYZED_ZYNTHIUM_ACID: "XZH2O",
	RESOURCE_CATALYZED_ZYNTHIUM_ALKALIDE: "XZHO2",
	RESOURCE_CATALYZED_GHODIUM_ACID: "XGH2O",
	RESOURCE_CATALYZED_GHODIUM_ALKALIDE: "XGHO2",
	// ── 商品 tier0 ──
	RESOURCE_SILICON: "silicon",
	RESOURCE_METAL: "metal",
	RESOURCE_BIOMASS: "biomass",
	RESOURCE_MIST: "mist",
	// ── 商品 tier1 ──
	RESOURCE_UTRIUM_BAR: "utrium_bar",
	RESOURCE_LEMERGIUM_BAR: "lergium_bar",
	RESOURCE_KEANIUM_BAR: "keanium_bar",
	RESOURCE_ZYNTHIUM_BAR: "zynthium_bar",
	RESOURCE_GHODIUM_MELT: "ghodium_melt",
	RESOURCE_OXIDANT: "oxidant",
	RESOURCE_REDUCTANT: "reductant",
	RESOURCE_PURIFIER: "purifier",
	RESOURCE_BATTERY: "battery",
	RESOURCE_WIRE: "wire",
	RESOURCE_SWITCH: "switch",
	RESOURCE_TRANSISTOR: "transistor",
	RESOURCE_MICROCHIP: "microchip",
	RESOURCE_CIRCUIT: "circuit",
	RESOURCE_DEVICE: "device",
	RESOURCE_CELL: "cell",
	RESOURCE_PHLEGM: "phlegm",
	RESOURCE_TISSUE: "tissue",
	RESOURCE_MUSCLE: "muscle",
	RESOURCE_ORGANOID: "organoid",
	RESOURCE_ALLOY: "alloy",
	RESOURCE_FIXTURE: "fixture",
	RESOURCE_FIXTURES: "fixtures",
	RESOURCE_FRAME: "frame",
	RESOURCE_HYDRAULICS: "hydraulics",
	RESOURCE_MACHINE: "machine",
	RESOURCE_CONDENSATE: "condensate",
	RESOURCE_CONCENTRATE: "concentrate",
	RESOURCE_EXTRACT: "extract",
	RESOURCE_SPIRIT: "spirit",
	RESOURCE_EMANATION: "emanation",
	RESOURCE_ESSENCE: "essence",
	RESOURCE_CRYSTAL: "crystal",
	// ── season 资源 ──
	RESOURCE_COMPOSITE: "composite",
	RESOURCE_LIQUID: "liquid",
	RESOURCE_ORGANISM: "organism",
	RESOURCE_PRODUCTION: "production",
	RESOURCE_TUBE: "tube",
	// ── 历史别名 (引擎仍保留) ──
	RESOURCE_ZYNTHIUM_KEANITE: "ZK",
	RESOURCE_UTRIUM_LEMERGITE: "UL",

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

	// ── 经济常量 (官方 constants.js 逐条对齐) ──
	CARRY_CAPACITY: 50,
	HARVEST_POWER: 2,
	HARVEST_MINERAL_POWER: 1,
	HARVEST_DEPOSIT_POWER: 1,
	BUILD_POWER: 5,
	REPAIR_POWER: 100,
	DISMANTLE_POWER: 50,
	UPGRADE_CONTROLLER_POWER: 1,
	ATTACK_POWER: 30,
	RANGED_ATTACK_POWER: 10,
	HEAL_POWER: 12,
	RANGED_HEAL_POWER: 4,
	REPAIR_COST: 0.01,
	DISMANTLE_COST: 0.005,
	CREEP_PART_MAX_ENERGY: 125,
	TOMBSTONE_DECAY_PER_PART: 5,
	ENERGY_DECAY: 1000,
	ENERGY_REGEN_TIME: 300,
	SOURCE_ENERGY_CAPACITY: 3000,
	SOURCE_ENERGY_NEUTRAL_CAPACITY: 1500,
	SOURCE_ENERGY_KEEPER_CAPACITY: 4000,
	SPAWN_HITS: 5000,
	SPAWN_ENERGY_START: 300,
	SPAWN_ENERGY_CAPACITY: 300,
	EXTENSION_HITS: 1000,
	EXTENSION_ENERGY_CAPACITY: { 0: 50, 1: 50, 2: 50, 3: 50, 4: 50, 5: 50, 6: 50, 7: 100, 8: 200 },
	LINK_HITS: 1000,
	LINK_CAPACITY: 800,
	LINK_COOLDOWN: 1,
	LINK_LOSS_RATIO: 0.03,
	STORAGE_HITS: 10000,
	STORAGE_CAPACITY: 1000000,
	CONTAINER_HITS: 250000,
	CONTAINER_CAPACITY: 2000,
	CONTAINER_DECAY: 5000,
	CONTAINER_DECAY_TIME: 100,
	CONTAINER_DECAY_TIME_OWNED: 500,
	TOWER_HITS: 3000,
	TOWER_CAPACITY: 1000,
	TOWER_ENERGY_COST: 10,
	TOWER_POWER_ATTACK: 600,
	TOWER_POWER_HEAL: 400,
	TOWER_POWER_REPAIR: 800,
	TOWER_OPTIMAL_RANGE: 5,
	TOWER_FALLOFF_RANGE: 20,
	TOWER_FALLOFF: 0.75,
	ROAD_HITS: 5000,
	ROAD_DECAY_AMOUNT: 100,
	ROAD_DECAY_TIME: 1000,
	ROAD_WEAROUT: 1,
	ROAD_WEAROUT_POWER_CREEP: 100,
	WALL_HITS: 1,
	WALL_HITS_MAX: 300000000,
	RAMPART_HITS: 1,
	RAMPART_HITS_MAX: { 2: 300000, 3: 1000000, 4: 3000000, 5: 10000000, 6: 30000000, 7: 100000000, 8: 300000000 },
	RAMPART_DECAY_AMOUNT: 300,
	RAMPART_DECAY_TIME: 100,
	TERMINAL_HITS: 3000,
	TERMINAL_CAPACITY: 300000,
	TERMINAL_SEND_COST: 0.1,
	TERMINAL_MIN_SEND: 100,
	OBSERVER_HITS: 500,
	POWER_SPAWN_HITS: 5000,
	POWER_SPAWN_ENERGY_CAPACITY: 5000,
	POWER_SPAWN_POWER_CAPACITY: 100,
	POWER_SPAWN_ENERGY_RATIO: 50,
	EXTRACTOR_HITS: 500,
	EXTRACTOR_COOLDOWN: 5,
	LAB_HITS: 500,
	LAB_MINERAL_CAPACITY: 3000,
	LAB_ENERGY_CAPACITY: 2000,
	LAB_BOOST_ENERGY: 20,
	LAB_BOOST_MINERAL: 30,
	LAB_COOLDOWN: 10,
	LAB_REACTION_AMOUNT: 5,
	NUKER_HITS: 1000,
	NUKER_COOLDOWN: 100000,
	NUKER_ENERGY_CAPACITY: 300000,
	NUKER_GHODIUM_CAPACITY: 5000,
	NUKE_LAND_TIME: 50000,
	NUKE_RANGE: 10,
	FACTORY_HITS: 1000,
	FACTORY_CAPACITY: 50000,
	MINERAL_REGEN_TIME: 50000,
	MINERAL_MIN_AMOUNT: { H: 35000, O: 35000, L: 35000, K: 35000, Z: 35000, U: 35000, X: 35000 },
	MINERAL_RANDOM_FACTOR: 2,
	MINERAL_DENSITY: { 1: 15000, 2: 35000, 3: 70000, 4: 100000 },
	MINERAL_DENSITY_PROBABILITY: { 1: 0.1, 2: 0.5, 3: 0.9, 4: 1.0 },
	MINERAL_DENSITY_CHANGE: 0.05,
	DEPOSIT_EXHAUST_MULTIPLY: 0.001,
	DEPOSIT_EXHAUST_POW: 1.6,
	DEPOSIT_DECAY_TIME: 50000,
	CONTROLLER_STRUCTURES: {
		spawn: { 0: 0, 1: 1, 2: 1, 3: 1, 4: 1, 5: 1, 6: 1, 7: 2, 8: 3 },
		extension: { 0: 0, 1: 0, 2: 5, 3: 10, 4: 20, 5: 30, 6: 40, 7: 50, 8: 60 },
		road: { 0: 2500, 1: 2500, 2: 2500, 3: 2500, 4: 2500, 5: 2500, 6: 2500, 7: 2500, 8: 2500 },
		constructedWall: { 0: 0, 1: 0, 2: 2500, 3: 2500, 4: 2500, 5: 2500, 6: 2500, 7: 2500, 8: 2500 },
		rampart: { 0: 0, 1: 0, 2: 2500, 3: 2500, 4: 2500, 5: 2500, 6: 2500, 7: 2500, 8: 2500 },
		link: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 2, 6: 3, 7: 4, 8: 6 },
		storage: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 1, 5: 1, 6: 1, 7: 1, 8: 1 },
		tower: { 0: 0, 1: 0, 2: 0, 3: 1, 4: 1, 5: 2, 6: 2, 7: 3, 8: 6 },
		observer: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 0, 8: 1 },
		powerSpawn: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 0, 8: 1 },
		extractor: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 1, 7: 1, 8: 1 },
		lab: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 3, 7: 6, 8: 10 },
		terminal: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 1, 7: 1, 8: 1 },
		container: { 0: 5, 1: 5, 2: 5, 3: 5, 4: 5, 5: 5, 6: 5, 7: 5, 8: 5 },
		nuker: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 0, 8: 1 },
		factory: { 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 1, 8: 1 },
	},

	// ── 事件 (官方 EVENT_*) ──
	EVENT_ATTACK: 1,
	EVENT_OBJECT_DESTROYED: 2,
	EVENT_ATTACK_CONTROLLER: 3,
	EVENT_BUILD: 4,
	EVENT_HARVEST: 5,
	EVENT_HEAL: 6,
	EVENT_REPAIR: 7,
	EVENT_RESERVE_CONTROLLER: 8,
	EVENT_UPGRADE_CONTROLLER: 9,
	EVENT_EXIT: 10,
	EVENT_TRANSFER: 11,
	EVENT_ATTACK_TYPE_MELEE: 1,
	EVENT_ATTACK_TYPE_RANGED: 2,
	EVENT_ATTACK_TYPE_RANGED_MASS: 3,
	EVENT_ATTACK_TYPE_DISMANTLE: 4,
	EVENT_ATTACK_TYPE_HIT_BACK: 5,
	EVENT_ATTACK_TYPE_NUKE: 6,
	EVENT_HEAL_TYPE_MELEE: 1,
	EVENT_HEAL_TYPE_RANGED: 2,

	// ── 世界 / 其他 ──
	PIXEL_CPU_COST: 10000,
	PIXEL_BUY_PRICE: 1,
	PIXEL_SELL_PRICE: 1,
	MARKET_MAX_ORDERS: 300,
	MARKET_ORDER_LIFE_TIME: 10000,
	MAX_CONSTRUCTION_SITES_PER_TICK: 1,
	ORDER_SELL: "sell",
	ORDER_BUY: "buy",
	TERRAIN_MASK_WALL: 1,
	TERRAIN_MASK_SWAMP: 2,
	TERRAIN_MASK_LAVA: 4,
	DIRECTION_TOP: 1,
	DIRECTION_TOP_RIGHT: 2,
	DIRECTION_RIGHT: 3,
	DIRECTION_BOTTOM_RIGHT: 4,
	DIRECTION_BOTTOM: 5,
	DIRECTION_BOTTOM_LEFT: 6,
	DIRECTION_LEFT: 7,
	DIRECTION_TOP_LEFT: 8,
	// ── 官方常量对齐 (@screeps/common constants) ──
	COLOR_PURPLE: 2,
	COLOR_BLUE: 3,
	COLOR_CYAN: 4,
	COLOR_GREEN: 5,
	COLOR_ORANGE: 7,
	COLOR_BROWN: 8,
	COLOR_GREY: 9,
	COLOR_WHITE: 10,
	WORLD_HEIGHT: 202,
	LINK_HITS_MAX: 1000,
	SAFE_MODE_DURATION: 20000,
	SAFE_MODE_COOLDOWN: 50000,
	SAFE_MODE_COST: 1000,
	OBSERVER_RANGE: 10,
	POWER_BANK_HITS: 2000000,
	POWER_BANK_CAPACITY_MAX: 5000,
	POWER_BANK_CAPACITY_MIN: 500,
	POWER_BANK_CAPACITY_CRIT: 0.3,
	POWER_BANK_DECAY: 5000,
	POWER_BANK_HIT_BACK: 0.5,
	LAB_UNBOOST_ENERGY: 0,
	LAB_UNBOOST_MINERAL: 15,
	GCL_NOVICE: 3,
	MODE_SIMULATION: null,
	MODE_WORLD: null,
	DENSITY_LOW: 1,
	DENSITY_MODERATE: 2,
	DENSITY_HIGH: 3,
	DENSITY_ULTRA: 4,
	TERMINAL_COOLDOWN: 10,
	NUKE_DAMAGE: { 0: 10000000, 2: 5000000 },
	TOMBSTONE_DECAY_POWER_CREEP: 500,
	RUIN_DECAY: 500,
	RUIN_DECAY_STRUCTURES: { 'powerBank': 10 },
	PORTAL_DECAY: 30000,
	MARKET_FEE: 0.05,
	FLAGS_LIMIT: 10000,
	SUBSCRIPTION_TOKEN: "token",
	CPU_UNLOCK: "cpuUnlock",
	PIXEL: "pixel",
	ACCESS_KEY: "accessKey",
	REACTIONS: { H: { O: "OH", L: "LH", K: "KH", U: "UH", Z: "ZH", G: "GH" }, O: { H: "OH", L: "LO", K: "KO", U: "UO", Z: "ZO", G: "GO" }, Z: { K: "ZK", H: "ZH", O: "ZO" }, L: { U: "UL", H: "LH", O: "LO" }, K: { Z: "ZK", H: "KH", O: "KO" }, G: { H: "GH", O: "GO" }, U: { L: "UL", H: "UH", O: "UO" }, OH: { UH: "UH2O", UO: "UHO2", ZH: "ZH2O", ZO: "ZHO2", KH: "KH2O", KO: "KHO2", LH: "LH2O", LO: "LHO2", GH: "GH2O", GO: "GHO2" }, X: { UH2O: "XUH2O", UHO2: "XUHO2", LH2O: "XLH2O", LHO2: "XLHO2", KH2O: "XKH2O", KHO2: "XKHO2", ZH2O: "XZH2O", ZHO2: "XZHO2", GH2O: "XGH2O", GHO2: "XGHO2" }, ZK: { UL: "G" }, UL: { ZK: "G" }, LH: { OH: "LH2O" }, ZH: { OH: "ZH2O" }, GH: { OH: "GH2O" }, KH: { OH: "KH2O" }, UH: { OH: "UH2O" }, LO: { OH: "LHO2" }, ZO: { OH: "ZHO2" }, KO: { OH: "KHO2" }, UO: { OH: "UHO2" }, GO: { OH: "GHO2" }, LH2O: { X: "XLH2O" }, KH2O: { X: "XKH2O" }, ZH2O: { X: "XZH2O" }, UH2O: { X: "XUH2O" }, GH2O: { X: "XGH2O" }, LHO2: { X: "XLHO2" }, UHO2: { X: "XUHO2" }, KHO2: { X: "XKHO2" }, ZHO2: { X: "XZHO2" }, GHO2: { X: "XGHO2" } },
	BOOSTS: { work: { UO: { harvest: 3 }, UHO2: { harvest: 5 }, XUHO2: { harvest: 7 }, LH: { build: 1.5, repair: 1.5 }, LH2O: { build: 1.8, repair: 1.8 }, XLH2O: { build: 2, repair: 2 }, ZH: { dismantle: 2 }, ZH2O: { dismantle: 3 }, XZH2O: { dismantle: 4 }, GH: { upgradeController: 1.5 }, GH2O: { upgradeController: 1.8 }, XGH2O: { upgradeController: 2 } }, attack: { UH: { attack: 2 }, UH2O: { attack: 3 }, XUH2O: { attack: 4 } }, ranged_attack: { KO: { rangedAttack: 2, rangedMassAttack: 2 }, KHO2: { rangedAttack: 3, rangedMassAttack: 3 }, XKHO2: { rangedAttack: 4, rangedMassAttack: 4 } }, heal: { LO: { heal: 2, rangedHeal: 2 }, LHO2: { heal: 3, rangedHeal: 3 }, XLHO2: { heal: 4, rangedHeal: 4 } }, carry: { KH: { capacity: 2 }, KH2O: { capacity: 3 }, XKH2O: { capacity: 4 } }, move: { ZO: { fatigue: 2 }, ZHO2: { fatigue: 3 }, XZHO2: { fatigue: 4 } }, tough: { GO: { damage: .7 }, GHO2: { damage: .5 }, XGHO2: { damage: .3 } } },
	REACTION_TIME: { OH: 20, ZK: 5, UL: 5, G: 5, UH: 10, UH2O: 5, XUH2O: 60, UO: 10, UHO2: 5, XUHO2: 60, KH: 10, KH2O: 5, XKH2O: 60, KO: 10, KHO2: 5, XKHO2: 60, LH: 15, LH2O: 10, XLH2O: 65, LO: 10, LHO2: 5, XLHO2: 60, ZH: 20, ZH2O: 40, XZH2O: 160, ZO: 10, ZHO2: 5, XZHO2: 60, GH: 10, GH2O: 15, XGH2O: 80, GO: 10, GHO2: 30, XGHO2: 150, },
	PORTAL_UNSTABLE: 10*24*3600*1000,
	PORTAL_MIN_TIMEOUT: 12*24*3600*1000,
	PORTAL_MAX_TIMEOUT: 22*24*3600*1000,
	POWER_BANK_RESPAWN_TIME: 50000,
	INVADERS_ENERGY_GOAL: 100000,
	SYSTEM_USERNAME: 'Screeps',
	SIGN_RESPAWN_AREA: 'A new Novice or Respawn Area is being planned somewhere in this sector. Please make sure all important rooms are reserved.',
	SIGN_PLANNED_AREA: 'A new Novice or Respawn Area is being planned somewhere in this sector. Please make sure all important rooms are reserved.',
	EVENT_POWER: 11,
	POWER_LEVEL_MULTIPLY: 1000,
	POWER_LEVEL_POW: 2,
	POWER_CREEP_SPAWN_COOLDOWN: 8*3600*1000,
	POWER_CREEP_DELETE_COOLDOWN: 24*3600*1000,
	POWER_CREEP_MAX_LEVEL: 25,
	POWER_CREEP_LIFE_TIME: 5000,
	POWER_CLASS: { OPERATOR: 'operator' },
	PWR_GENERATE_OPS: 1,
	PWR_OPERATE_SPAWN: 2,
	PWR_OPERATE_TOWER: 3,
	PWR_OPERATE_STORAGE: 4,
	PWR_OPERATE_LAB: 5,
	PWR_OPERATE_EXTENSION: 6,
	PWR_OPERATE_OBSERVER: 7,
	PWR_OPERATE_TERMINAL: 8,
	PWR_DISRUPT_SPAWN: 9,
	PWR_DISRUPT_TOWER: 10,
	PWR_DISRUPT_SOURCE: 11,
	PWR_SHIELD: 12,
	PWR_REGEN_SOURCE: 13,
	PWR_REGEN_MINERAL: 14,
	PWR_DISRUPT_TERMINAL: 15,
	PWR_OPERATE_POWER: 16,
	PWR_FORTIFY: 17,
	PWR_OPERATE_CONTROLLER: 18,
	PWR_OPERATE_FACTORY: 19,
	EFFECT_INVULNERABILITY: 1001,
	EFFECT_COLLAPSE_TIMER: 1002,
	INVADER_CORE_HITS: 100000,
	INVADER_CORE_CREEP_SPAWN_TIME: { 0: 0, 1: 0, 2: 6, 3: 3, 4: 2, 5: 1 },
	INVADER_CORE_EXPAND_TIME: { 1: 4000, 2: 3500, 3: 3000, 4: 2500, 5: 2000 },
	INVADER_CORE_CONTROLLER_POWER: 2,
	INVADER_CORE_CONTROLLER_DOWNGRADE: 5000,
	STRONGHOLD_RAMPART_HITS: { 0: 0, 1: 100000, 2: 200000, 3: 500000, 4: 1000000, 5: 2000000 },
	STRONGHOLD_DECAY_TICKS: 75000,
};
for (const k of Object.keys(C)) {
	globalThis[k] = C[k];
}
globalThis.RESOURCES_ALL = C.RESOURCES_ALL;

// ═══════════════════════════ lodash ═══════════════════════════
// 官方 runtime 把完整 lodash 3.10.1 注入为全局 `_` (E/game/game.js:475-477)。
// 宿主 (runtime.rs) 会在装载本 shim 之前先执行 src/js/lodash.js, 因此这里不再自带桩实现。
if (typeof globalThis._ === "undefined" || typeof globalThis._.chain !== "function") {
	throw new Error("screeps.js shim 需要宿主先注入 lodash 3.10.1 为全局 `_`");
}
// 官方沙箱提供 `global` 别名 (D/runtime/user-vm.js:59); bundle 常写 global.xxx
globalThis.global = globalThis;

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
/** 本 tick 已创建的 creep 名 (官方 createdCreepNames), 每 tick 重建 */
let __createdCreepNames = new Set();
/** 本 tick 已创建的工地数 (官方把同 tick 新建计入 100 上限) */
let __createdSiteCount = 0;

/** 意图的 CPU 计价 (官方 driver lib/runtime/runtime.js: intentCpu = 0.2) */
const INTENT_CPU = 0.2;
/** 免计价的意图 (官方 freeMethods: say / pull) */
const FREE_INTENT_METHODS = { say: true, pull: true };
/** 本 tick 已计入 CPU 的 (对象, 意图) 组合 —— 官方 intents.set 只对首次出现计价 */
let __intentKeys = new Set();
let __intentCpu = 0;

function intent(objId, name, data) {
	const o = typeof objId === "string" ? objId : String(objId);
	__intents.push({ o, n: name, d: data });
	// 官方: 同一对象同一意图重复提交不重复计价 (intents.set 语义)
	if (!FREE_INTENT_METHODS[name]) {
		const key = o + "|" + name;
		if (!__intentKeys.has(key)) {
			__intentKeys.add(key);
			__intentCpu += INTENT_CPU;
		}
	}
}

/** 未实现 API 的统一错误 (issue #1: 战斗/claim 等未覆盖链路, 明确报错而不是 TypeError) */
function unimplemented(name) {
	return new Error(`${name} 未实现: 本模拟器仅覆盖经济链路 (采集/搬运/建造/修理/升级/孵化)`);
}

/** 官方 utils.calcCreepCost: 身体部件总造价 */
function creepCost(body) {
	let sum = 0;
	for (const p of body) sum += C.BODYPART_COST[p.type] || 0;
	return sum;
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
	/** 官方 E/game/rooms.js:1586-1592: 跨房恒为 Infinity; 支持 (x, y) 重载 */
	getRangeTo(target, y) {
		const pos = arguments.length >= 2
			? { x: target, y, roomName: this.roomName }
			: (target && target.pos ? target.pos : target);
		if (!pos || pos.x === undefined) return NaN;
		if (pos.roomName && pos.roomName !== this.roomName) return Infinity;
		return Math.max(Math.abs(this.x - pos.x), Math.abs(this.y - pos.y));
	}
	inRangeTo(target, y, range) {
		if (arguments.length >= 3) return this.getRangeTo(target, y) <= range;
		return this.getRangeTo(target) <= y;
	}
	isEqualTo(target, y) {
		const pos = typeof y === "number" ? { x: target, y, roomName: this.roomName } : (target && target.pos ? target.pos : target);
		if (!pos) return false;
		return this.x === pos.x && this.y === pos.y && (!pos.roomName || this.roomName === pos.roomName);
	}
	/** 官方 E/game/rooms.js:1378-1381: `<= 1` (站在目标格上也算 near) */
	isNearTo(target) {
		return this.getRangeTo(target) <= 1;
	}
	/** 官方 E/game/rooms.js:1383-1394: 用世界坐标判定方向 (跨房也正确) */
	getDirectionTo(target, y) {
		const pos = arguments.length >= 2
			? { x: target, y, roomName: this.roomName }
			: (target && target.pos ? target.pos : target);
		const [ax, ay] = roomNameToXY(this.roomName);
		const [bx, by] = roomNameToXY(pos.roomName || this.roomName);
		return getDirection(bx * 50 + pos.x - (ax * 50 + this.x), by * 50 + pos.y - (ay * 50 + this.y));
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
// 官方 E/game/store.js 用 Proxy 实现:
//   - 只枚举非零资源 (内部字段/方法不可枚举)
//   - RESOURCES_ALL 中缺失的键读取返回 0
//   - getUsedCapacity(res) 返回该资源的数量; 不传参数才返回总和

const RESOURCES_ALL_SET = new Set(C.RESOURCES_ALL);
const storeMeta = new WeakMap();

function storeSum(target) {
	let sum = 0;
	for (const k of Object.keys(target)) sum += target[k];
	return sum;
}

/** 官方 utils.capacityForResource: 专用格 (storeCapacityResource) 优先, 否则是总容量减去专用格之和 */
function capacityForResource(meta, resource) {
	if (meta.capacities && meta.capacities[resource]) return meta.capacities[resource];
	let reserved = 0;
	if (meta.capacities) for (const k in meta.capacities) reserved += meta.capacities[k];
	return Math.max(0, (meta.capacity || 0) - reserved);
}

/** 官方 E/game/store.js getCapacity: 专用仓不传资源返回 null, 容量为 0 也返回 null */
function storeCapacityOf(target, resource) {
	const meta = storeMeta.get(target);
	if (!meta) return null;
	if (!resource) return meta.capacities ? null : meta.capacity || null;
	return capacityForResource(meta, resource) || null;
}

function storeUsedOf(target, resource) {
	const meta = storeMeta.get(target) || {};
	if (!resource) {
		if (meta.capacities && capacityForResource(meta, resource) === 0) return null;
		return storeSum(target);
	}
	return target[resource] || (meta.capacities && capacityForResource(meta, resource) === 0 ? null : 0);
}

function storeFreeOf(target, resource) {
	const meta = storeMeta.get(target) || {};
	if (capacityForResource(meta, resource) === 0) return null;
	if (!meta.capacity) return storeCapacityOf(target, resource) - storeUsedOf(target, resource);
	const capacity = storeCapacityOf(target, resource);
	if (!capacity) return null;
	if (meta.capacities) return capacity - storeUsedOf(target, resource);
	return capacity - storeSum(target);
}

const storeHandler = {
	get(target, prop, receiver) {
		switch (prop) {
			case "getCapacity":
				return (resource) => storeCapacityOf(target, resource);
			case "getUsedCapacity":
				return (resource) => storeUsedOf(target, resource);
			case "getFreeCapacity":
				return (resource) => storeFreeOf(target, resource);
			case "toJSON":
				return () => Object.assign({}, target);
			default:
				break;
		}
		if (typeof prop === "string" && prop in target) return target[prop];
		if (typeof prop === "string" && RESOURCES_ALL_SET.has(prop)) return 0;
		return Reflect.get(target, prop, receiver);
	},
	has(target, prop) {
		if (typeof prop === "string" && RESOURCES_ALL_SET.has(prop)) return true;
		return Reflect.has(target, prop);
	},
	set(target, prop, value) {
		if (typeof prop === "string" && RESOURCES_ALL_SET.has(prop)) {
			const v = +value || 0;
			if (v > 0) target[prop] = v;
			else delete target[prop];
			return true;
		}
		return Reflect.set(target, prop, value);
	},
	deleteProperty(target, prop) {
		return Reflect.deleteProperty(target, prop);
	},
};

function makeStore(raw, storeCapacity, capacities) {
	const data = {};
	for (const k of Object.keys(raw || {})) {
		const v = +raw[k];
		if (v > 0) data[k] = v;
	}
	storeMeta.set(data, {
		capacity: storeCapacity == null ? 0 : storeCapacity,
		capacities: capacities || null,
	});
	return new Proxy(data, storeHandler);
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
		if (!this.__storeObj) this.__storeObj = makeStore(this.__storeRaw, this.__storeCap, null);
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
	notifyWhenAttacked(enabled) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (typeof enabled !== "boolean") return C.ERR_INVALID_ARGS;
		return C.OK;
	}
	/** 官方 E/game/creeps.js:126-156: move(creep) 跟着相邻 creep 走 (配合 pull), 不查疲劳/MOVE 部件 */
	move(direction) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (direction && direction instanceof Creep) {
			if (!direction.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
			intent(this.id, "move", { id: direction.id });
			return C.OK;
		}
		if (this.fatigue > 0) return C.ERR_TIRED;
		if (!this.getActiveBodyparts("move")) return C.ERR_NO_BODYPART;
		direction = +direction;
		if (!direction || direction < 1 || direction > 8) return C.ERR_INVALID_ARGS;
		intent(this.id, "move", { direction });
		return C.OK;
	}
	moveByPath(path) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (Array.isArray(path) && path.length > 0 && path[0] instanceof RoomPosition) {
			let idx = path.findIndex((p) => p.isEqualTo(this.pos));
			if (idx === -1 && !path[0].isNearTo(this.pos)) return C.ERR_NOT_FOUND;
			idx++;
			if (idx >= path.length) return C.ERR_NOT_FOUND;
			return this.move(this.pos.getDirectionTo(path[idx]));
		}
		// 官方 E/game/creeps.js:305-330: 字符串先反序列化, 非数组返回 ERR_INVALID_ARGS
		if (typeof path === "string") path = deserializePath(path);
		if (!Array.isArray(path)) return C.ERR_INVALID_ARGS;
		const cur = path.find((i) => i.x - i.dx === this.pos.x && i.y - i.dy === this.pos.y);
		if (!cur) return C.ERR_NOT_FOUND;
		return this.move(cur.direction);
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
				// 官方 rooms.js moveTo: 只按 (x,y) 匹配缓存步, 序列化步没有 roomName
				const idx = path.findIndex((p) => p.x === this.pos.x && p.y === this.pos.y);
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
	/** 官方 E/game/creeps.js:428-490 的返回码顺序 */
	transfer(target, resourceType, amount) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (amount < 0) return C.ERR_INVALID_ARGS;
		if (!C.RESOURCES_ALL.includes(resourceType)) return C.ERR_INVALID_ARGS;
		// 官方只拒绝「正在孵化的 creep」; 正在孵化的 spawn (spawning 为 Spawning 对象) 照样能收货
		if (!target || !target.id || (target instanceof Creep && target.spawning)) return C.ERR_INVALID_TARGET;
		if (!(target instanceof Creep) && !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (target instanceof StructureController) {
			if (resourceType === C.RESOURCE_ENERGY) return this.upgradeController(target);
			return C.ERR_INVALID_TARGET;
		}
		if (!target.store) return C.ERR_INVALID_TARGET;
		const capacity = target.store.getCapacity(resourceType);
		if (!capacity) return C.ERR_INVALID_TARGET;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		const have = this.store[resourceType];
		if (!have) return C.ERR_NOT_ENOUGH_RESOURCES;
		// 单资源仓 (spawn/extension/tower/lab…) 按该资源计, 通用仓 (creep/container/storage) 按总量计
		const stored = target.store.getCapacity() === null ? target.store[resourceType] : target.store.getUsedCapacity();
		if (stored >= capacity) return C.ERR_FULL;
		if (!amount) amount = Math.min(have, capacity - stored);
		if (have < amount) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (amount + stored > capacity) return C.ERR_FULL;
		intent(this.id, "transfer", { id: target.id, resourceType, amount });
		return C.OK;
	}
	withdraw(target, resourceType, amount) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!C.RESOURCES_ALL.includes(resourceType)) return C.ERR_INVALID_ARGS;
		if (amount !== undefined && amount < 0) return C.ERR_INVALID_ARGS;
		if (!target || !target.id || !target.store) return C.ERR_INVALID_TARGET;
		const available = target.store.getUsedCapacity(resourceType);
		if (amount === undefined) amount = available;
		if (!available) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		if (this.store.getFreeCapacity() <= 0) return C.ERR_FULL;
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
		if (amount !== undefined && amount < 0) return C.ERR_INVALID_ARGS;
		const store = this.store;
		const have = store[resourceType];
		if (!have) return C.ERR_NOT_ENOUGH_RESOURCES;
		if (amount === undefined) amount = have;
		// 官方 drop.js:13: 数量超过持有量返回错误而不是截断
		if (amount > have) return C.ERR_NOT_ENOUGH_RESOURCES;
		intent(this.id, "drop", { amount, resourceType });
		return C.OK;
	}
	build(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!target || !(target instanceof ConstructionSite)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target.pos, 3)) return C.ERR_NOT_IN_RANGE;
		// 官方 creeps.js build: 障碍类建筑的格子上有 creep 或其他障碍 → ERR_INVALID_TARGET
		if (C.OBSTACLE_OBJECT_TYPES.includes(target.structureType)) {
			const cell = this.room.__occupied.get(`${target.pos.x},${target.pos.y}`) || [];
			const blocked = cell.some(
				(o) => !(o instanceof ConstructionSite) && (o instanceof Creep || C.OBSTACLE_OBJECT_TYPES.includes(o.structureType))
			);
			if (blocked) return C.ERR_INVALID_TARGET;
		}
		intent(this.id, "build", { id: target.id });
		return C.OK;
	}
	repair(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!target || !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target.pos, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "repair", { id: target.id });
		return C.OK;
	}
	upgradeController(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
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
	// ── 战斗 (官方 E/game/creeps.js:593-732) ──
	/** 官方 attack: ATTACK 部件, 近战 1 格 */
	attack(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("attack")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Creep) && !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "attack", { id: target.id, x: target.pos.x, y: target.pos.y });
		return C.OK;
	}
	/** 官方 rangedAttack: RANGED_ATTACK 部件, 3 格 */
	rangedAttack(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("ranged_attack")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Creep) && !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "rangedAttack", { id: target.id });
		return C.OK;
	}
	/** 官方 rangedMassAttack: RANGED_ATTACK 部件, 无参群体攻击 */
	rangedMassAttack() {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("ranged_attack")) return C.ERR_NO_BODYPART;
		intent(this.id, "rangedMassAttack", {});
		return C.OK;
	}
	/** 官方 heal: HEAL 部件, 近战 1 格 */
	heal(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("heal")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Creep)) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "heal", { id: target.id, x: target.pos.x, y: target.pos.y });
		return C.OK;
	}
	/** 官方 rangedHeal: HEAL 部件, 3 格 */
	rangedHeal(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("heal")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Creep)) return C.ERR_INVALID_TARGET;
		if (!this.pos.inRangeTo(target, 3)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "rangedHeal", { id: target.id });
		return C.OK;
	}
	/** 官方 dismantle: WORK 部件, 目标必须是可建造结构 (有建造成本) */
	dismantle(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!this.getActiveBodyparts("work")) return C.ERR_NO_BODYPART;
		if (!target || !target.id || !(target instanceof Structure) || !C.CONSTRUCTION_COST[target.structureType]) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "dismantle", { id: target.id });
		return C.OK;
	}
	/** 官方 attackController: CLAIM 部件, 目标须有 owner 或 reservation */
	attackController(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof StructureController)) return C.ERR_INVALID_TARGET;
		if (!this.getActiveBodyparts("claim")) return C.ERR_NO_BODYPART;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		if (!target.my && !target.reservation) return C.ERR_INVALID_TARGET;
		if (target.upgradeBlocked > 0) return C.ERR_TIRED;
		intent(this.id, "attackController", { id: target.id });
		return C.OK;
	}
	/** 官方 claimController: CLAIM 部件, GCL 检查 (官方按已占领房间数) */
	claimController(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof StructureController)) return C.ERR_INVALID_TARGET;
		if (!this.getActiveBodyparts("claim")) return C.ERR_NO_BODYPART;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "claimController", { id: target.id });
		return C.OK;
	}
	/** 官方 reserveController: CLAIM 部件, 目标须无主且无他人预约 */
	/** 官方 E/game/creeps.js:955-985 */
	reserveController(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		if (target.structureType !== "controller") return C.ERR_INVALID_TARGET;
		if (target.owner) return C.ERR_INVALID_TARGET;
		// 自己的保留可以续 (此前误用 !target.my, 中立控制器恒为 false, 续保留永远被拒)
		if (target.reservation && target.reservation.username !== this.owner.username) return C.ERR_INVALID_TARGET;
		if (!this.getActiveBodyparts("claim")) return C.ERR_NO_BODYPART;
		intent(this.id, "reserveController", { id: target.id });
		return C.OK;
	}
	/** 官方 signController: 任意 creep 均可签名 (官方无部件/归属检查) */
	signController(target, sign) {
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof StructureController)) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "signController", { id: target.id, sign: "" + sign });
		return C.OK;
	}
	/** 官方 E/game/creeps.js:1093-1113 */
	pull(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof Creep) || !target.my && !Game.getObjectById(target.id) || target.spawning || target.id === this.id) return C.ERR_INVALID_TARGET;
		if (!target.pos.isNearTo(this.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "pull", { id: target.id });
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
		this.store = makeStore(raw.store, Infinity, null);
		this.ticksToDecay = raw.ticksToDecay;
		this.creep = raw.creepName ? { name: raw.creepName } : null;
		this.deathTime = 0;
	}
}

/**
 * 官方各建筑的 store 形状 (storeCapacity / storeCapacityResource), 决定 getCapacity()/getFreeCapacity() 的语义:
 * 只存能量的建筑是专用仓 (不传资源时 getCapacity() 为 null), container/storage/terminal/factory 是通用仓。
 */
function structureStoreShape(type, cap) {
	switch (type) {
		case "spawn":
		case "extension":
		case "tower":
		case "link":
			return [null, { energy: cap }];
		case "lab":
			return [C.LAB_MINERAL_CAPACITY + C.LAB_ENERGY_CAPACITY, { energy: C.LAB_ENERGY_CAPACITY }];
		case "powerSpawn":
			return [null, { energy: C.POWER_SPAWN_ENERGY_CAPACITY, power: C.POWER_SPAWN_POWER_CAPACITY }];
		case "nuker":
			return [null, { energy: C.NUKER_ENERGY_CAPACITY, G: C.NUKER_GHODIUM_CAPACITY }];
		default:
			return [cap, null];
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
		this.__off = !!raw.off;
	}
	get store() {
		if (!this.__storeObj && this.__storeRaw) {
			const [cap, caps] = structureStoreShape(this.structureType, this.__storeCap);
			this.__storeObj = makeStore(this.__storeRaw, cap, caps);
		}
		return this.__storeObj;
	}
	/** 官方 isActive(): 超出 RCL 上限被停用的 spawn/extension 返回 false */
	isActive() {
		return !this.__off;
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

class OwnedStructure extends Structure {
	constructor(raw) {
		super(raw);
		// 官方 E/game/structures.js:135-140: OwnedStructure 有 owner: {username}
		this.owner = this.my ? { username: "Nexus" } : undefined;
	}
}
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
		if (!Memory.spawns[this.name]) Memory.spawns[this.name] = {};
		return Memory.spawns[this.name];
	}
	set memory(v) {
		if (!Memory.spawns) Memory.spawns = {};
		Memory.spawns[this.name] = v;
	}
	/** 官方 E/game/structures.js:1073-1128 */
	spawnCreep(body, name, opts) {
		opts = opts || {};
		if (this.spawning) return C.ERR_BUSY;
		if (typeof name !== "string" || !name || name.length > 100) return C.ERR_INVALID_ARGS;
		if (Game.creeps[name] || __createdCreepNames.has(name)) return C.ERR_NAME_EXISTS;
		if (!Array.isArray(body) || body.length === 0) return C.ERR_INVALID_ARGS;
		// 官方: 超过 50 个部件返回 ERR_INVALID_ARGS (不再静默截断)
		if (body.length > C.MAX_CREEP_SIZE) return C.ERR_INVALID_ARGS;
		let cost = 0;
		for (const p of body) {
			if (!C.BODYPART_COST[p]) return C.ERR_INVALID_ARGS;
			cost += C.BODYPART_COST[p];
		}
		if (cost > this.room.energyAvailable) return C.ERR_NOT_ENOUGH_ENERGY;
		// dryRun: 副作用 (意图/Memory) 之前返回
		if (opts.dryRun) return C.OK;
		if (!Memory.creeps) Memory.creeps = {};
		// 官方总是创建 Memory.creeps[name], 未传 opts.memory 时为 {}
		Memory.creeps[name] = opts.memory || {};
		__createdCreepNames.add(name);
		const data = { body, name };
		if (opts.directions) data.directions = opts.directions;
		if (opts.energyStructures) data.energyStructures = opts.energyStructures.map((s) => s.id || s);
		intent(this.id, "spawnCreep", data);
		return C.OK;
	}
	/** 官方 StructureSpawn.setDirections: 覆盖正在孵化单位的出生方向 */
	setDirections(name, directions) {
		if (this.spawning && this.spawning.name === name) {
			intent(this.id, "setDirections", { name, directions });
			return C.OK;
		}
		return C.ERR_NOT_FOUND;
	}
	canCreateCreep() {
		return this.spawning ? C.ERR_BUSY : C.OK;
	}
	/** 官方 E/game/structures.js:1237-1266 */
	renewCreep(target) {
		if (this.spawning) return C.ERR_BUSY;
		if (!target || !target.id || !(target instanceof Creep) || target.spawning) return C.ERR_INVALID_TARGET;
		if (!this.my || !target.my) return C.ERR_NOT_OWNER;
		if (this.__off) return C.ERR_RCL_NOT_ENOUGH;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		const len = target.body.length;
		const cost = Math.ceil((C.SPAWN_RENEW_RATIO * creepCost(target.body)) / C.CREEP_SPAWN_TIME / len);
		if (this.room.energyAvailable < cost) return C.ERR_NOT_ENOUGH_ENERGY;
		const effect = Math.floor((C.SPAWN_RENEW_RATIO * C.CREEP_LIFE_TIME) / C.CREEP_SPAWN_TIME / len);
		if (target.ticksToLive + effect > C.CREEP_LIFE_TIME) return C.ERR_FULL;
		intent(this.id, "renewCreep", { id: target.id });
		return C.OK;
	}
	/** 官方 E/game/structures.js:1269-1290: 回收单位, 资源 100% 掉落 */
	recycleCreep(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (!target || !target.id || !(target instanceof Creep) || target.spawning) return C.ERR_INVALID_TARGET;
		if (this.__off) return C.ERR_RCL_NOT_ENOUGH;
		if (!target.my) return C.ERR_NOT_OWNER;
		if (!this.pos.isNearTo(target.pos)) return C.ERR_NOT_IN_RANGE;
		intent(this.id, "recycleCreep", { id: target.id });
		return C.OK;
	}
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
		// 官方 structures.js: 没有保留时为 undefined; 字段名 ticksToEnd
		this.reservation = raw.reservation || undefined;
		this.sign = raw.sign || null;
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
	/** 官方 E/game/structures.js:766-785 */
	attack(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (!target || !target.id || !(target instanceof Creep) && !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!this.store || this.store.energy < C.TOWER_ENERGY_COST) return C.ERR_NOT_ENOUGH_ENERGY;
		intent(this.id, "attack", { id: target.id });
		return C.OK;
	}
	/** 官方 E/game/structures.js:786-805 */
	heal(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (!target || !target.id || !(target instanceof Creep)) return C.ERR_INVALID_TARGET;
		if (!this.store || this.store.energy < C.TOWER_ENERGY_COST) return C.ERR_NOT_ENOUGH_ENERGY;
		intent(this.id, "heal", { id: target.id });
		return C.OK;
	}
	/** 官方 E/game/structures.js:806-828 */
	repair(target) {
		if (!this.my) return C.ERR_NOT_OWNER;
		if (!target || !target.id || !(target instanceof Structure)) return C.ERR_INVALID_TARGET;
		if (!this.store || this.store.energy < C.TOWER_ENERGY_COST) return C.ERR_NOT_ENOUGH_ENERGY;
		intent(this.id, "repair", { id: target.id });
		return C.OK;
	}
}
class StructureLink extends OwnedStructure {
	constructor(raw) {
		super(raw);
		this.energy = raw.store ? raw.store.energy || 0 : 0;
		this.energyCapacity = raw.energyCapacity;
		this.cooldown = raw.cooldown || 0;
	}
	/** 官方 E/game/structures.js:488-540 */
	transferEnergy(target, amount) {
		if (amount < 0) return C.ERR_INVALID_ARGS;
		if (!target || !target.id || !(target instanceof Structure) || target === this || target.structureType !== C.STRUCTURE_LINK) {
			return C.ERR_INVALID_TARGET;
		}
		if (!target.my) return C.ERR_NOT_OWNER;
		if (this.cooldown > 0) return C.ERR_TIRED;
		if (!this.room.controller) return C.ERR_RCL_NOT_ENOUGH;
		if (!this.store || !this.store.energy) return C.ERR_NOT_ENOUGH_ENERGY;
		if (!amount) amount = Math.min(this.store.energy, (target.energyCapacity || 0) - (target.energy || 0));
		if (this.store.energy < amount) return C.ERR_NOT_ENOUGH_ENERGY;
		intent(this.id, "transferEnergy", { id: target.id, amount });
		return C.OK;
	}
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
	/** 官方 E/game/flags.js:31: flag 也有 memory (自动建对象 + setter) */
	get memory() {
		if (!Memory.flags) Memory.flags = {};
		if (!Memory.flags[this.name]) Memory.flags[this.name] = {};
		return Memory.flags[this.name];
	}
	set memory(v) {
		if (!Memory.flags) Memory.flags = {};
		Memory.flags[this.name] = v;
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
		this.__byType = { creep: [], source: [], mineral: [], energy: [], constructionSite: [], tombstone: [], flag: [], powerCreep: [] };
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
			// 官方 E/game/structures.js:45-55: 任何控制器/storage/terminal 都挂到房间上 (不限归属)
			if (obj instanceof StructureController) this.controller = obj;
			if (obj instanceof StructureStorage) this.storage = obj;
			if (obj instanceof StructureTerminal) this.terminal = obj;
		}
		this.energyAvailable = raw.energyAvailable;
		this.energyCapacityAvailable = raw.energyCapacityAvailable;
	}
	get memory() {
		if (!Memory.rooms) Memory.rooms = {};
		if (!Memory.rooms[this.name]) Memory.rooms[this.name] = {};
		return Memory.rooms[this.name];
	}
	set memory(v) {
		if (!Memory.rooms) Memory.rooms = {};
		Memory.rooms[this.name] = v;
	}
	getTerrain() {
		return new Terrain(this.name);
	}
	getPositionAt(x, y) {
		if (x < 0 || x > 49 || y < 0 || y > 49) return null;
		return new RoomPosition(x, y, this.name);
	}
	/** 官方 E/game/rooms.js:648-650: `_.filter(result, opts.filter)` (函数/对象/字符串均可) */
	find(type, opts) {
		return collectFind(this.name, type, opts);
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
	/** 官方 Room.lookAtArea(top, left, bottom, right, [asArray]) */
	lookAtArea(top, left, bottom, right, asArray) {
		const out = asArray ? [] : {};
		for (let y = top; y <= bottom; y++) {
			for (let x = left; x <= right; x++) {
				const entries = this.lookAt(x, y);
				if (asArray) {
					for (const e of entries) out.push({ x, y, ...e });
				} else {
					if (!out[y]) out[y] = {};
					out[y][x] = entries;
				}
			}
		}
		return out;
	}
	/** 官方 Room.lookForAtArea(type, top, left, bottom, right, [asArray]) */
	lookForAtArea(type, top, left, bottom, right, asArray) {
		const out = asArray ? [] : {};
		for (let y = top; y <= bottom; y++) {
			for (let x = left; x <= right; x++) {
				const entries = this.lookForAt(type, x, y);
				if (asArray) {
					for (const e of entries) out.push({ x, y, [type]: e });
				} else {
					if (!out[y]) out[y] = {};
					out[y][x] = entries;
				}
			}
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
		if (Game.constructionSites && Object.keys(Game.constructionSites).length + __createdSiteCount >= C.MAX_CONSTRUCTION_SITES) return C.ERR_FULL;
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
		// 官方只豁免 road/extractor: 容器不允许建在墙上
		if (structureType !== "road" && structureType !== "extractor") {
			const t = terrainOf(this.name)[y * 50 + x];
			if (t & C.TERRAIN_MASK_WALL) return C.ERR_INVALID_TARGET;
		}
		intent("room", "createConstructionSite", { room: this.name, x, y, structureType, name });
		__createdSiteCount++;
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
	findPath(fromPos, toPos, opts) {
		return roomFindPathImpl(this.name, fromPos, toPos, opts);
	}
	findExitTo(room) {
		return Game.map.findExit(this.name, typeof room === "string" ? room : room.name);
	}
	/** 官方 E/game/rooms.js:573-582: 返回上一 tick 的房间事件; raw=true 时返回 JSON 字符串 */
	getEventLog(raw) {
		const events = (this.__raw && this.__raw.events) || [];
		if (raw) return JSON.stringify(events);
		return events.slice();
	}
	toString() {
		return `[room ${this.name}]`;
	}
}
Room.Terrain = Terrain;
// 官方 E/game/rooms.js:528-534: 静态方法 (此前误做成实例方法)
Room.serializePath = function (path) {
	return serializePath(path);
};
Room.deserializePath = function (str) {
	return deserializePath(str);
};

function wrapObject(room, raw) {
	raw.roomName = room.name;
	switch (raw.type) {
		case "creep": return new Creep(raw);
		case "source": return new Source(raw);
		case "energy": return new Resource(raw);
		case "constructionSite": return new ConstructionSite(raw);
		case "tombstone": return new Tombstone(raw);
		case "mineral": return new Mineral(raw);
		case "deposit": return new Deposit(raw);
		case "ruin": return new Ruin(raw);
		case "nuke": return new Nuke(raw);
		case "powerCreep": return new PowerCreep(raw);
		default: {
			const cls = STRUCTURE_CLASSES[raw.type];
			if (!cls) return null;
			const obj = new cls({ ...raw, structureType: raw.type });
			return obj;
		}
	}
}

/** 官方 `_.filter(result, opts.filter)`: 支持函数 / 对象 (matches) / 字符串 (属性简写) */
function applyFilter(arr, filter) {
	if (filter === undefined || filter === null) return arr;
	return _.filter(arr, filter);
}

function collectFind(roomName, type, opts) {
	const room = Game.rooms[roomName];
	if (!room) return [];
	const filter = opts && opts.filter;
	const pick = (arr) => applyFilter(arr, filter);
	switch (type) {
		case C.FIND_CREEPS: return pick(room.__byType.creep);
		case C.FIND_MY_CREEPS: return pick(room.__byType.creep.filter((c) => c.my));
		case C.FIND_HOSTILE_CREEPS: return pick(room.__byType.creep.filter((c) => !c.my));
		case C.FIND_SOURCES: return pick(room.__byType.source);
		case C.FIND_MINERALS: return pick(room.__byType.mineral || []);
		case C.FIND_SOURCES_ACTIVE: return pick(room.__byType.source.filter((s) => s.energy > 0));
		case C.FIND_DROPPED_RESOURCES: return pick(room.__byType.energy);
		case C.FIND_STRUCTURES: return pick(room.__objects.filter((o) => o instanceof Structure));
		case C.FIND_MY_STRUCTURES: return pick(room.__objects.filter((o) => o instanceof OwnedStructure && o.my));
		case C.FIND_CONSTRUCTION_SITES:
		case C.FIND_MY_CONSTRUCTION_SITES: return pick(room.__byType.constructionSite.filter((s) => s.my));
		case C.FIND_TOMBSTONES: return pick(room.__byType.tombstone);
		case C.FIND_FLAGS: return pick(Object.values(Game.flags).filter((f) => f.pos.roomName === roomName));
		case C.FIND_MY_SPAWNS: return pick(room.__byType.spawn.filter((s) => s.my));
		case C.FIND_HOSTILE_SPAWNS: return pick(room.__byType.spawn.filter((s) => !s.my));
		case C.FIND_EXIT_TOP: return pick(exitPositions(roomName, 1));
		case C.FIND_EXIT_RIGHT: return pick(exitPositions(roomName, 3));
		case C.FIND_EXIT_BOTTOM: return pick(exitPositions(roomName, 5));
		case C.FIND_EXIT_LEFT: return pick(exitPositions(roomName, 7));
		case C.FIND_EXIT: {
			const exits = [];
			for (const dir of [1, 3, 5, 7]) exits.push(...exitPositions(roomName, dir));
			return pick(exits);
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
	// path 模式: 多目标 PathFinder (官方 E/game/rooms.js:337-373)
	const goals = objects.map((o) => ({ pos: o.pos || o, range: 1 }));
	const searchOpts = buildMoveSearchOpts(pos.roomName, opts);
	// 官方 findClosestByPath 默认 maxRooms: 1
	if (searchOpts.maxRooms === undefined) searchOpts.maxRooms = 1;
	const result = pathFinderSearch(pos, goals, searchOpts);
	// 路径为空 (已相邻) 时以起点为 lastPos; 部分路径也照常返回最近对象
	const last = result.path.length ? result.path[result.path.length - 1] : pos;
	let best = null;
	let bestRange = Infinity;
	for (const o of objects) {
		const r = last.getRangeTo(o.pos || o);
		if (r < bestRange) {
			bestRange = r;
			best = o;
		}
	}
	return best;
}

/** moveTo/findPath 的搜索参数 (官方 _findPath2: 默认考虑道路, plain 2 / swamp 10) */
function buildMoveSearchOpts(roomName, opts) {
	opts = opts || {};
	// 官方 E/game/rooms.js:163,169-171: 仅当 opts.ignoreCreeps 为真才忽略 creep, 默认 false
	const ignoreCreeps = opts.ignoreCreeps !== undefined ? opts.ignoreCreeps : false;
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
	// 无视野房间: 官方用只有地形的空 CostMatrix (E/game/rooms.js:200-202), 不是不可走
	if (!Game.rooms[roomName]) return new CostMatrix();
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
// 官方 E/utils.js:555-613: "XXYY" (两位 x + 两位 y) 后接每一步的 direction 单字符。

function serializePath(path) {
	if (!Array.isArray(path)) throw new Error("path is not an array");
	let result = "";
	if (!path.length) return result;
	if (path[0].x < 0 || path[0].y < 0) throw new Error("path coordinates cannot be negative");
	result += path[0].x > 9 ? path[0].x : "0" + path[0].x;
	result += path[0].y > 9 ? path[0].y : "0" + path[0].y;
	for (let i = 0; i < path.length; i++) result += path[i].direction;
	return result;
}
function deserializePath(str) {
	if (typeof str !== "string") throw new Error("`path` is not a string");
	const result = [];
	if (!str.length) return result;
	let x = parseInt(str.substring(0, 2));
	let y = parseInt(str.substring(2, 4));
	if (isNaN(x) || isNaN(y)) throw new Error("`path` is not a valid serialized path string");
	for (let i = 4; i < str.length; i++) {
		const direction = parseInt(str.charAt(i));
		if (!OFFSETS[direction]) throw new Error("`path` is not a valid serialized path string");
		const dx = OFFSETS[direction][0], dy = OFFSETS[direction][1];
		if (i > 4) { x += dx; y += dy; }
		result.push({ x, y, dx, dy, direction });
	}
	return result;
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
	constructor() {
		// 官方 E/game/path-finder.js:19-31: 内部为列优先 x*50+y 的 Uint8Array(2500)
		this.__bits = new Uint8Array(2500);
	}
	set(x, y, value) {
		x = x | 0;
		y = y | 0;
		// 官方 clamp 到 [0,255]: set(256) → 255 (不再是溢出成 0)
		this.__bits[x * 50 + y] = Math.min(Math.max(0, value), 255);
		return this;
	}
	get(x, y) {
		x = x | 0;
		y = y | 0;
		return this.__bits[x * 50 + y];
	}
	clone() {
		const m = new CostMatrix();
		m.__bits = new Uint8Array(this.__bits);
		return m;
	}
	/** 官方 serialize: 返回 Uint32Array(625) 的普通数组 (即 2500 字节的视图) */
	serialize() {
		return Array.prototype.slice.call(new Uint32Array(this.__bits.buffer));
	}
	/** 官方 deserialize(array): 接受 serialize() 的数组 (此前误实现为按逗号拆字符串) */
	static deserialize(data) {
		const m = new CostMatrix();
		m.__bits = new Uint8Array(new Uint32Array(data).buffer);
		return m;
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
	// 目标归一化: 接受 RoomPosition 或 {pos, range} (官方 path-finder.js:58-60)
	const goals = (Array.isArray(goal) ? goal : [goal])
		.map((g) => {
			if (!g) return null;
			if (g.pos) return { pos: g.pos, range: g.range || 0 };
			return { pos: g, range: 0 };
		})
		.filter((g) => g && g.pos && g.pos.roomName);
	// 空目标数组: 官方直接返回, 不消耗 ops
	if (goals.length === 0) return { path: [], ops: 0, cost: 0, incomplete: false };
	const plainCost = opts.plainCost || 1;
	const swampCost = opts.swampCost || 5;
	const maxRooms = Math.max(1, opts.maxRooms || OP_LIMIT_ROOMS);
	const maxOps = opts.maxOps || OP_LIMIT_OPS;
	const maxCost = opts.maxCost || Infinity;
	const hw = opts.heuristicWeight === undefined ? 1.2 : opts.heuristicWeight;
	const flee = !!opts.flee;

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
		// CostMatrix 内部列优先 x*50+y (官方 path-finder.js)
		const m = room.matrix ? room.matrix[x * 50 + y] : 0;
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
	// flee: 目标是要远离的东西, 评分 = 到最近 goal 的切比雪夫世界距离 (越大越好)
	function fleeScore(roomIdx, x, y) {
		const room = roomData[roomIdx];
		const wx = room.rx * 50 + x, wy = room.ry * 50 + y;
		let best = Infinity;
		for (const a of goalAnchors) {
			const d = Math.max(Math.abs(wx - a.wx), Math.abs(wy - a.wy)) - a.range;
			if (d < best) best = d;
		}
		return best;
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
	// 官方: 搜索不完整时返回通往启发值最低节点的部分路径
	let bestNode = null;
	let bestH = Infinity;
	// flee 模式: 记录 "离目标最远" 的节点 (评分最高, 同分取路径最短)
	let bestFlee = -Infinity;
	let bestFleeG = Infinity;
	if (!flee && inGoal(originIdx, origin.x, origin.y)) found = { room: originIdx, x: origin.x, y: origin.y };

	while ((flee || !found) && heap.size && ops < maxOps) {
		const node = heap.pop();
		ops++;
		const { room: ridx, x, y } = node;
		const ci = y * 50 + x;
		if (closed[ridx][ci]) continue;
		closed[ridx][ci] = 1;
		if (node.g > gCost[ridx][ci]) continue;
		if (flee) {
			const s = fleeScore(ridx, x, y);
			if (s > bestFlee || (s === bestFlee && node.g < bestFleeG)) {
				bestFlee = s;
				bestFleeG = node.g;
				bestNode = { room: ridx, x, y };
			}
		} else {
			const h = heuristic(ridx, x, y);
			if (h < bestH) {
				bestH = h;
				bestNode = { room: ridx, x, y };
			}
			if (inGoal(ridx, x, y)) { found = { room: ridx, x, y }; break; }
		}

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
	const end = flee ? bestNode : found || bestNode;
	if (end) {
		let { room: ridx, x, y } = end;
		cost = gCost[ridx][y * 50 + x];
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
	return { path, ops, cost, incomplete: flee ? ops >= maxOps : !found };
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
// Memory: 官方是 globalThis 上的**可配置**属性 (delete global.Memory 不抛错,
// issue #1 §3.4)。var 创建的属性 configurable:false 且无法重定义, 故用访问器。
var __memoryValue = null;
Object.defineProperty(globalThis, "Memory", {
	configurable: true,
	enumerable: true,
	get() {
		return __memoryValue;
	},
	set(v) {
		__memoryValue = v;
	},
});

function buildGame() {
	byId = new Map();
	__createdCreepNames = new Set();
	__createdSiteCount = 0;
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
			// 官方 E/game/game.js:302-305: Game.structures 只含自己的 OwnedStructure
			if (obj instanceof OwnedStructure && obj.my) Game.structures[obj.id] = obj;
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

/** 由官方 CONTROLLER_STRUCTURES 派生 (不再手写一份易漂移的表) */
function structureLimitsFor(rcl) {
	const out = {};
	const lv = Math.min(rcl, 8);
	for (const [st, table] of Object.entries(C.CONTROLLER_STRUCTURES)) out[st] = table[lv];
	return out;
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
		getUsed() { return nowMs() - start + __intentCpu; },
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
				for (const [dir, name] of Object.entries(this.describeExits(cur) || {})) {
					// 官方: Number(cb(roomName, fromRoomName)) || 1; 返回 Infinity 表示不可进入
					let cost = 1;
					if (cb) cost = Number(cb(name, cur)) || 1;
					const nd = curD + cost;
					if (nd < (dist.get(name) ?? Infinity)) {
						dist.set(name, nd);
						prev.set(name, { from: cur, exit: +dir });
					}
				}
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
			// 支持 (x, y, roomName) 与 (pos) 两种重载; 返回 'wall'/'swamp'/'plain' (官方为字符串)
			if (typeof x === "object" && x !== null) { roomName = x.roomName; y = x.y; x = x.x; }
			const t = new Terrain(roomName).get(x, y);
			return t & C.TERRAIN_MASK_WALL ? "wall" : t & C.TERRAIN_MASK_SWAMP ? "swamp" : "plain";
		},
		isRoomAvailable(roomName) {
			return __W.roomNames.includes(roomName);
		},
		getRoomStatus(roomName) {
			// 官方恒返回 {status, timestamp}; 场景外房间也一样 (此前返回 null)
			if (typeof roomName !== "string" || !/^([WE])\d+([NS])\d+$/.test(roomName)) return null;
			return { status: "normal", timestamp: null };
		},
		visual: new RoomVisual("map"),
	};
}

// ═══════════════════════════ 计时 / 随机 / 控制台 ═══════════════════════════

const __ops = globalThis.Deno && globalThis.Deno.core ? globalThis.Deno.core.ops : null;
const runtimeStart = __ops && __ops.op_sim_now ? __ops.op_sim_now() : 0;
// 确定性 CPU (场景 cpu.deterministic / --deterministic-cpu): 不读墙钟, 每次取时间推进固定步长,
// Game.cpu.getUsed() 与本 tick cpuUsed 只由调用次数和意图数决定 → 同一场景每次运行逐位一致。
let __virtualCpuStep = 0;
let __virtualClock = 0;
const __realDateNow = Date.now;
function nowMs() {
	if (__virtualCpuStep > 0) {
		__virtualClock += __virtualCpuStep;
		return __virtualClock;
	}
	return __ops && __ops.op_sim_now ? __ops.op_sim_now() - runtimeStart : __realDateNow() - runtimeStart;
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
	log: (...args) => __consoleLine(args),
	info: (...args) => __consoleLine(args),
	warn: (...args) => __consoleLine(args),
	error: (...args) => __consoleLine(args),
};
/** 控制台输出: 同时写本地缓冲 (正常路径) 与宿主缓冲 (CPU 中断后仍可见日志) */
function __consoleLine(args) {
	const line = args.map(fmt).join(" ");
	__consoleLines.push(line);
	if (__ops && __ops.op_sim_console) {
		__ops.op_sim_console(line);
	}
}
globalThis.console = console;
function fmt(v) {
	if (typeof v === "string") return v;
	try { return JSON.stringify(v); } catch { return String(v); }
}

// ═══════════════════════════ RawMemory / 全局暴露 ═══════════════════════════

let rawMemoryData = "";
/** 本 tick 可读写的内存段 (宿主注入) */
let __activeSegments = [];
/** 本 tick setActiveSegments 指定的"下一 tick 激活列表" (官方: 不调用即全部停用) */
let __nextActiveSegments = null;
let __publicSegments = null;
let __defaultPublicSegment = null;
let __activeForeignSegment = null;

const MAX_MEMORY_SEGMENTS = 10;
const SEGMENT_MAX_LENGTH = 100 * 1024;
const RAW_MEMORY_MAX_LENGTH = 2 * 1024 * 1024;

/** 段 ID 校验 (官方 runtime.js: 0..99, 否则抛错) */
function __segmentId(id) {
	const n = parseInt(id);
	if (isNaN(n) || n < 0 || n > 99) {
		throw new Error(`"${id}" is not a valid segment ID`);
	}
	return n;
}

const RawMemory = {
	get() { return rawMemoryData; },
	set(value) {
		// 官方: 非字符串 / 超过 2MB 直接抛错
		if (typeof value !== "string") throw new Error("Raw memory value is not a string");
		if (value.length > RAW_MEMORY_MAX_LENGTH) throw new Error("Raw memory length exceeded 2 MB limit");
		rawMemoryData = value;
		// 官方 Memory 是 rawMemory 的惰性解析视图: set 之后立即反映新内容
		try {
			globalThis.Memory = value ? JSON.parse(value) : {};
		} catch {
			globalThis.Memory = {};
		}
	},
	segments: {},
	foreignSegment: null,
	interShardSegment: "",
	setActiveSegments(ids) {
		if (!Array.isArray(ids)) throw new Error(`"${ids}" is not an array`);
		if (ids.length > MAX_MEMORY_SEGMENTS) {
			throw new Error("Only 10 memory segments can be active at the same time");
		}
		__nextActiveSegments = ids.map(__segmentId);
	},
	setPublicSegments(ids) {
		if (!Array.isArray(ids)) throw new Error(`"${ids}" is not an array`);
		__publicSegments = ids.map(__segmentId);
	},
	setDefaultPublicSegment(id) {
		__defaultPublicSegment = id === null ? null : __segmentId(id);
	},
	setActiveForeignSegment(username, id) {
		if (username === null) {
			__activeForeignSegment = null;
			return;
		}
		__activeForeignSegment = { username, id: id === undefined ? undefined : __segmentId(id) };
	},
};
const InterShardMemory = {
	getLocal() { return ""; },
	setLocal() {},
	getRemote() { return ""; },
};

// ── 独立类型 (此前都别名为 RoomObject, 导致 `x instanceof Mineral` 恒为真) ──
class Mineral extends RoomObject {
	constructor(raw) {
		super(raw);
		this.mineralType = raw.mineralType;
		this.mineralAmount = raw.mineralAmount;
		this.density = raw.density;
		this.ticksToRegeneration = raw.ticksToRegeneration;
	}
}
class Deposit extends RoomObject {
	constructor(raw) {
		super(raw);
		this.depositType = raw.depositType;
		this.cooldown = raw.cooldown || 0;
		this.ticksToDecay = raw.ticksToDecay;
	}
}
class Ruin extends RoomObject {
	constructor(raw) {
		super(raw);
		this.store = makeStore(raw.store, Infinity, null);
		this.structure = raw.structureType ? { structureType: raw.structureType } : undefined;
		this.ticksToDecay = raw.ticksToDecay;
	}
}
class Nuke extends RoomObject {
	constructor(raw) {
		super(raw);
		this.timeToLand = raw.timeToLand;
		this.launchRoomName = raw.launchRoomName;
	}
}
class PowerCreep extends RoomObject {
	constructor(raw) {
		super(raw);
		this.name = raw.name;
		this.my = raw.my !== false;
		this.owner = { username: "Nexus" };
		this.store = makeStore(raw.store, Infinity, null);
		this.hits = raw.hits;
		this.hitsMax = raw.hitsMax;
		this.ticksToLive = raw.ticksToLive;
	}
}

/** 全局类导出: 真实实现的直接挂载, 未实现的挂占位基类 */
const GLOBAL_CLASSES = {
	RoomPosition, Room, RoomObject, RoomVisual, Creep, Source, Resource,
	ConstructionSite, Tombstone, Flag, Structure, OwnedStructure,
	StructureSpawn, StructureController, StructureExtension, StructureStorage,
	StructureContainer, StructureTerminal, StructureTower, StructureLink, StructureLab,
	StructureNuker, StructureObserver, StructurePowerSpawn, StructureExtractor,
	StructureFactory, StructureRoad, StructureWall, StructureRampart,
	Ruin, Mineral, Deposit, Nuke, PowerCreep,
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
	// 官方 E/game/game.js:503-510,543-548: lodash 可 require, 未知模块抛 "Unknown module"
	globalThis.__require = (name) => {
		if (name === "lodash") return globalThis._;
		throw new Error(`Unknown module: ${name}`);
	};
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
	__intentKeys = new Set();
	__intentCpu = 0;
	__consoleLines = [];
	try {
		// bot 上一 tick 可能 delete global.Memory: 每 tick 确保访问器存在 (官方由 runtime 重建)
		if (!Object.getOwnPropertyDescriptor(globalThis, "Memory")) {
			Object.defineProperty(globalThis, "Memory", {
				configurable: true,
				enumerable: true,
				get() {
					return __memoryValue;
				},
				set(v) {
					__memoryValue = v;
				},
			});
		}
		// 运行时数据先取 (确定性 CPU 要在 buildGame → makeCpu 取起点之前生效)
		let rt = {};
		try {
			rt = JSON.parse((__ops.op_sim_get_runtime && __ops.op_sim_get_runtime()) || "{}");
		} catch {
			rt = {};
		}
		__virtualCpuStep = +rt.virtualCpuStep || 0;
		// 确定性模式下 Date.now 跟 Game.time 走 (每 tick 3 秒), 不读真实时间
		Date.now = __virtualCpuStep > 0 ? () => 1600000000000 + (__W ? __W.time : 0) * 3000 : __realDateNow;
		__W = JSON.parse(__ops.op_sim_get_snapshot());
		buildGame();
		rawMemoryData = __ops.op_sim_get_memory();
		try {
			globalThis.Memory = rawMemoryData ? JSON.parse(rawMemoryData) : {};
		} catch {
			globalThis.Memory = {};
		}
		// 内存段: 宿主注入本 tick 激活的段 (官方 driver 的 data.memorySegments)
		__nextActiveSegments = null;
		__publicSegments = null;
		__defaultPublicSegment = null;
		__activeForeignSegment = null;
		RawMemory.segments = {};
		try {
			__activeSegments = rt.activeSegments || [];
			for (const id of __activeSegments) {
				RawMemory.segments[id] = (rt.segments || {})[id] || "";
			}
		} catch {
			__activeSegments = [];
		}
		globalThis.Game = Game;

		const loop = __botModule && __botModule.exports && __botModule.exports.loop;
		if (typeof loop === "function") {
			const t0 = nowMs();
			try {
				loop();
			} catch (e) {
				errors.push(e && e.stack ? String(e.stack) : String(e));
			}
			// 官方 usedTime = ceil(nowCpuTime() - startTime + intents.cpu): 意图计价计入本 tick CPU
			cpuUsed = Math.ceil(nowMs() - t0 + __intentCpu);
		} else {
			errors.push("bot bundle 未导出 loop 函数");
		}
	} catch (e) {
		errors.push(e && e.stack ? String(e.stack) : String(e));
	}
	let memoryOut = "";
	try {
		// 官方: 读 rawMemory._parsed (delete global.Memory 只删属性, 底层数据保留并照常保存)
		memoryOut = JSON.stringify(__memoryValue || {});
	} catch {
		memoryOut = "{}";
	}
	// 官方 saveUserMemory: 序列化后超过 2MB 拒绝保存 (memory 置 null → 宿主保留上一 tick 的值)
	if (memoryOut.length > RAW_MEMORY_MAX_LENGTH) {
		errors.push("Script execution has been terminated: memory allocation limit reached");
		memoryOut = null;
	}
	// 内存段写回 + 校验 (官方 runtime.js 结尾: ≤10 段 / id 0..99 / 必须字符串 / ≤100KB)
	let memorySegments;
	const segKeys = Object.keys(RawMemory.segments);
	if (segKeys.length > 0) {
		if (segKeys.length > MAX_MEMORY_SEGMENTS) {
			errors.push("Cannot save more than 10 memory segments on the same tick");
		} else {
			const out = {};
			for (const k of segKeys) {
				const key = parseInt(k);
				const value = RawMemory.segments[k];
				if (isNaN(key) || key < 0 || key > 99) {
					errors.push(`"${k}" is not a valid memory segment ID`);
					break;
				}
				if (typeof value !== "string") {
					errors.push(`Memory segment #${k} is not a string`);
					break;
				}
				if (value.length > SEGMENT_MAX_LENGTH) {
					errors.push(`Memory segment #${k} has exceeded 100 KB length limit`);
					break;
				}
				out[key] = value;
			}
			if (Object.keys(out).length === segKeys.length) memorySegments = out;
		}
	}
	const result = JSON.stringify({
		intents: __intents,
		memory: memoryOut,
		cpuUsed,
		errors,
		console: __consoleLines.slice(0, 50),
		memorySegments,
		// 官方: 本 tick 未调用 setActiveSegments 即视为下一 tick 无激活段
		activeSegments: __nextActiveSegments || [],
		publicSegments: __publicSegments,
		defaultPublicSegment: __defaultPublicSegment,
		activeForeignSegment: __activeForeignSegment,
	});
	__ops.op_sim_result(result);
	return result;
}

function __seedRandom(seed) {
	seedRandom(seed >>> 0);
}
