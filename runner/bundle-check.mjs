// bot bundle (dist/main.js) 静态检查: 在模拟前指出"能被静态发现"的错误。
// 与场景检查 (check.mjs) 互补 —— 本模块检查的是将被 V8 执行的 bot 代码。
//
// 检查分三级语义:
//   ✗ error   几乎必然在运行时炸掉 (拼写错误的常量 / node 泄漏 / 参数顺序颠倒)
//   ⚠ warn    能跑但行为可疑 (异步 API 跨 tick 必死 / 模拟器未覆盖的 API)
//
// 实现为零依赖的词法扫描 (行过滤注释 + 词边界正则), 不做完整 AST ——
// 目标是高置信度的少量规则, 而不是通用 linter (那是 eslint/tsc 的职责)。

import { SCREEPS_CONSTANTS } from './screeps-constants.mjs'

// 疑似官方常量的前缀 (命中前缀但不在常量表 → 拼写错误)
const CONSTANT_PREFIXES = [
	'OK',
	'ERR_',
	'FIND_',
	'RESOURCE_',
	'ORDER_',
	'BODY_',
	'CONTROLLER_',
	'STRUCTURE_',
	'TERRAIN_',
	'LOOK_',
	'COLOR_',
	'CREEP_',
	'SPAWN_',
	'TOWER_',
	'LAB_',
	'GCL_',
	'GPL_',
	'EVENT_',
	'POWER_',
	'MARKET_',
	'TERMINAL_',
	'NUKE_',
	'ENERGY_',
	'SOURCE_',
	'REACTION_',
	'BOOST_',
	'PORTAL_',
	'DEPOSIT_',
]

// node/浏览器环境泄漏: screeps 运行时 (官方与模拟器一致) 没有这些
const NODE_GLOBALS = [
	/\brequire\s*\(/, // 无模块系统 (bundler 未内联干净)
	/\bprocess\./, // process.env.NODE_ENV 残留 → 生产也会炸
	/\bBuffer\./,
	/\b__dirname\b/,
	/\bmodule\.require\b/,
]

// 定时器/网络: 官方运行时不存在 (bot 常见的迁移期错误)
const MISSING_APIS = [
	[/\bsetTimeout\s*\(/, 'setTimeout 在 screeps 运行时不存在 (跨 tick 定时请用 Game.time 状态机)'],
	[/\bsetInterval\s*\(/, 'setInterval 在 screeps 运行时不存在'],
	[/\bsetImmediate\s*\(/, 'setImmediate 在 screeps 运行时不存在'],
	[/\bfetch\s*\(/, 'fetch 在 screeps 运行时不存在 (无网络)'],
	[/\bXMLHttpRequest\b/, 'XMLHttpRequest 在 screeps 运行时不存在 (无网络)'],
]

// 模拟器覆盖度: bot 用到但模拟器不模拟的 API
// [匹配, 说明, 级别] —— level: 'error' 调用即抛错 / 'warn' no-op 或恒定值
const SIM_COVERAGE = [
	[/\bGame\.market\b/, '市场为 no-op 桩: 订单永远为空, deal 永远失败 —— 市场行为不会被模拟', 'warn'],
	[/\bGame\.powerCreeps\b/, 'powerCreeps 恒为空对象 —— power creep 不会被模拟', 'warn'],
	[/\bGame\.gpl\b/, 'GPL 恒为 0 —— power 进阶不会被模拟', 'warn'],
	[/\brunReaction\s*\(/, '实验室反应未实现 (StructureLab 为空类, 调用即 TypeError)', 'error'],
	[/\bboostCreep\s*\(/, 'boost 未实现 (方法缺失, 调用即 TypeError) —— boost 数值不会被模拟', 'error'],
	[/\bunboostCreep\s*\(/, 'boost 未实现 (方法缺失, 调用即 TypeError)', 'error'],
	[/\blaunchNuke\s*\(/, 'nuker 未实现 (StructureNuker 为空类, 调用即 TypeError)', 'error'],
	[/\bterminal\.send\s*\(/, 'terminal.send 未实现 (StructureTerminal 为空类, 调用即 TypeError) —— 跨房交易不会被模拟', 'error'],
	[/\bCreep\.prototype\.pull\b|\b\.pull\s*\(/, 'Creep.pull 未实现 (明确抛错)', 'error'],
	[/\bproduce\s*\(/, 'factory.produce 为 no-op (恒返回 ERR_INVALID_ARGS)', 'warn'],
	[/\bprocessPower\s*\(/, 'powerSpawn.processPower 为 no-op', 'warn'],
	[/\bobserveRoom\s*\(/, 'observer.observeRoom 为 no-op', 'warn'],
]

// 资源字符串 (transfer/withdraw 第一参数若为资源 → 顺序颠倒了)
const RESOURCE_LITERAL = /^(energy|[KUZLXOH](?:[KUZLXOH]{1,2})?|power|ops|silicon|metal|biomass|mist|utrium|lemergium|zynthium|keanium|oxidant|hydrogen|ghodium)$/

/**
 * @param {string} source bundle 源码
 * @param {{filename?: string}} opts
 * @returns {{errors: Array<{line:number,message:string}>, warnings: Array<{line:number,message:string}>}}
 */
export function checkBundle(source, opts = {}) {
	const filename = opts.filename ?? 'bundle'
	/** @type {Array<{line:number,message:string}>} */
	const errors = []
	/** @type {Array<{line:number,message:string}>} */
	const warnings = []
	const lines = source.split('\n')
	const isComment = (text) => /^\s*(\/\/|\*|\/\*|<!--)/.test(text)

	// ── node 泄漏 / 缺失 API (跳过注释行, 降误报) ──
	lines.forEach((line, i) => {
		if (isComment(line)) return
		const n = i + 1
		for (const re of NODE_GLOBALS) {
			if (re.test(line)) errors.push({ line: n, message: `检测到 ${re.source} —— screeps 运行时无 node 环境 (官方同样会炸, 检查构建配置的 define/external)` })
		}
		for (const [re, msg] of MISSING_APIS) {
			if (re.test(line)) warnings.push({ line: n, message: msg })
		}
	})

	// ── 模拟器覆盖度 ──
	lines.forEach((line, i) => {
		if (isComment(line)) return
		for (const [re, msg, level] of SIM_COVERAGE) {
			if (re.test(line)) {
				;(level === 'error' ? errors : warnings).push({ line: i + 1, message: msg })
			}
		}
	})

	// ── 官方常量拼写 (命中前缀但不在常量表) ──
	const identRe = /\b[A-Z][A-Z0-9_]{2,}\b/g
	lines.forEach((line, i) => {
		if (isComment(line)) return
		for (const m of line.matchAll(identRe)) {
			const tok = m[0]
			if (SCREEPS_CONSTANTS.has(tok)) continue
			if (CONSTANT_PREFIXES.some((p) => tok === p || tok.startsWith(p))) {
				errors.push({ line: i + 1, message: `常量 ${tok} 不在官方常量表中 —— 疑似拼写错误 (运行时为 undefined, 比较永远失败)` })
			}
		}
	})

	// ── 参数顺序颠倒 (高频事故: 官方签名 target 在前 / body 在前) ──
	const callRe = /\.(spawnCreep|transfer|withdraw)\s*\(\s*([^,()]{1,60})/g
	lines.forEach((line, i) => {
		if (isComment(line)) return
		for (const m of line.matchAll(callRe)) {
			const [ , method, firstArg] = m
			const trimmed = firstArg.trim().replace(/;$/, '')
			const n = i + 1
			if (method === 'spawnCreep' && /^['"]/.test(trimmed)) {
				errors.push({ line: n, message: `spawnCreep('${trimmed}', …) 第一参数是字符串 —— 官方签名 spawnCreep(body, name), body 数组在前` })
			}
			if ((method === 'transfer' || method === 'withdraw') && (/^RESOURCE_/.test(trimmed) || (/^['"]/.test(trimmed) && RESOURCE_LITERAL.test(trimmed.replace(/^['"]|['"]$/g, ''))))) {
				errors.push({ line: n, message: `${method}(${trimmed}, …) 第一参数是资源 —— 官方签名 ${method}(target, resourceType), target 在前` })
			}
		}
	})

	// ── loop 导出 (启发式: 常见导出形态都没有 → 大概率跑不起来) ──
	if (!/exports\.loop\b|module\.exports\s*=|"loop"|'loop'|\.loop\s*=/.test(source)) {
		warnings.push({ line: 0, message: '未发现 loop 导出 (exports.loop / module.exports = {loop}) —— 引擎将报 "bot bundle 未导出 loop 函数"' })
	}

	// ── 跨 tick 异步: promise 在 tick 结束后不会再被调度 ──
	lines.forEach((line, i) => {
		if (isComment(line)) return
		if (/\bWebAssembly\.instantiate\s*\(/.test(line)) {
			warnings.push({ line: i + 1, message: 'WebAssembly.instantiate 是异步 API —— promise 在 tick 结束后死亡, 永不 resolve。改用同步的 new WebAssembly.Module/Instance' })
		}
	})

	return { errors, warnings }
}
