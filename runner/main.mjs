// .sim.ts 测试 runner: 发现用例文件 → Node 原生加载 TS → 每用例一个引擎进程 → 汇总报告。
//
// 管道协议: 场景 JSON 走引擎 stdin, 结构化报告 (含断言结果) 从引擎 stdout 收,
// 人类可读输出与 bot 错误在引擎 stderr (失败时按需展示)。
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { ensureBinary, runBinaryCapture, runBinaryAsync } from './binary.mjs'
import { toWireScenario } from './wire.mjs'
import { checkScenario } from './check.mjs'
import { checkBundle } from './bundle-check.mjs'

const SIM_FILE_RE = /\.sim\.(ts|mts|cts|js|mjs|cjs)$/
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', '.cargo', 'dist', 'build', 'coverage'])

// ── 参数解析 ──────────────────────────────────────────────

export function parseTestArgs(args) {
	const opts = { patterns: [], bundle: undefined, ticks: undefined, verbose: false, list: false, strict: false, jobs: 1, bundleCheck: true }
	for (let i = 0; i < args.length; i++) {
		const a = args[i]
		switch (a) {
			case '--bundle':
			case '-b':
				opts.bundle = resolve(args[++i])
				break
			case '--ticks':
			case '-t':
				opts.ticks = Number(args[++i])
				break
			case '--verbose':
			case '-v':
				opts.verbose = true
				break
			case '--list':
			case '-l':
				opts.list = true
				break
			case '--strict':
				opts.strict = true
				break
			case '--no-bundle-check':
				opts.bundleCheck = false
				break
			case '--jobs':
			case '-j':
				opts.jobs = Math.max(1, Number(args[++i]) || 1)
				break
			default:
				if (a.startsWith('-')) throw new Error(`test 子命令未知参数: ${a}`)
				opts.patterns.push(a)
		}
	}
	return opts
}

// ── 用例文件发现 ──────────────────────────────────────────

export function discoverSimFiles(patterns, cwd) {
	const targets = patterns.length ? patterns : ['.']
	const files = []
	const seen = new Set()
	const push = (f) => {
		const p = resolve(cwd, f)
		if (!seen.has(p)) {
			seen.add(p)
			files.push(p)
		}
	}
	for (const t of targets) {
		const abs = resolve(cwd, t)
		if (existsSync(abs) && statSync(abs).isFile()) {
			push(abs)
			continue
		}
		if (/[*?]/.test(t)) {
			for (const f of matchGlob(abs)) push(f)
			continue
		}
		if (existsSync(abs) && statSync(abs).isDirectory()) {
			walk(abs, push)
			continue
		}
		throw new Error(`路径不存在: ${t}`)
	}
	return files.filter((f) => SIM_FILE_RE.test(f))
}

function walk(dir, out) {
	let entries
	try {
		entries = readdirSync(dir, { withFileTypes: true })
	} catch {
		return
	}
	for (const ent of entries) {
		const p = join(dir, ent.name)
		if (ent.isDirectory()) {
			if (!SKIP_DIRS.has(ent.name)) walk(p, out)
		} else if (SIM_FILE_RE.test(ent.name)) {
			out(p)
		}
	}
}

/// 极简 glob: `*` 匹配单层, `**` 匹配多层 (够用即可, 不引依赖)
function matchGlob(absPattern) {
	const out = []
	const re = new RegExp(
		'^' +
			absPattern
				.replace(/[.+^${}()|[\]\\]/g, '\\$&')
				.replace(/\*\*/g, '\u0000')
				.replace(/\*/g, '[^/]*')
				.replace(/\?/g, '[^/]')
				.replace(/\u0000/g, '.*') +
			'$'
	)
	// 从模式中找最深的存在前缀目录开始扫
	const parts = absPattern.split('/')
	let base = '/'
	for (let i = 0; i < parts.length; i++) {
		if (/[*?]/.test(parts[i])) break
		base = parts.slice(0, i + 1).join('/') || '/'
	}
	walk(base, (f) => {
		if (re.test(f)) out.push(f)
	})
	return out
}

// ── 用例加载 ──────────────────────────────────────────────

/// 一个 .sim.ts 文件可导出多个用例: default 导出 + 所有"长得像场景"的具名导出。
/// 不 import defineSim 也可以: 纯对象导出 (name + rooms) 同样被识别, 零依赖可用。
async function loadCases(file) {
	let mod
	try {
		mod = await import(pathToFileURL(file).href)
	} catch (err) {
		if (err?.code === 'ERR_UNKNOWN_FILE_EXTENSION') {
			throw new Error(
				`Node ${process.version} 无法直接加载 TS 文件 (需要 ≥ 22.6): ${file}\n` +
					'  升级 Node, 或改用 .sim.mjs / .sim.js'
			)
		}
		if (err instanceof SyntaxError && /export|import|Unexpected token/i.test(err.message)) {
			throw new Error(
				`加载失败: ${file}\n` +
					`  该文件被 Node 当作 CommonJS 解析 (项目 package.json 未设 "type": "module")。\n` +
					'  解决: ① 文件改名为 .sim.mts ② sim/ 目录放一个 {"type":"module"} 的 package.json\n' +
					'        ③ 项目 package.json 加 "type": "module"\n' +
					`  原始错误: ${err.message}`
			)
		}
		throw err
	}
	const cases = []
	const push = (value, exportName) => {
		if (!looksLikeCase(value)) return
		cases.push({ sim: { ...value }, exportName, file })
	}
	if (mod.default) push(mod.default, 'default')
	for (const [k, v] of Object.entries(mod)) {
		if (k === 'default') continue
		push(v, k)
	}
	return cases
}

function looksLikeCase(v) {
	return (
		v !== null &&
		typeof v === 'object' &&
		typeof v.name === 'string' &&
		v.name.length > 0 &&
		Array.isArray(v.rooms) &&
		v.rooms.length > 0
	)
}

// ── 项目级配置 (bot 项目的 package.json → "screeps-sim" 字段) ──

/// {
///   "screeps-sim": {
///     "bundle": "dist/main.js",       // 默认 bundle (相对项目根)
///     "include": ["sim"]              // 默认扫描范围 (替代无参数时的当前目录)
///   }
/// }
export function readProjectConfig(cwd) {
	try {
		const pkg = JSON.parse(readFileSync(join(cwd, 'package.json'), 'utf8'))
		const cfg = pkg['screeps-sim']
		if (!cfg || typeof cfg !== 'object') return {}
		return {
			bundle: typeof cfg.bundle === 'string' ? resolve(cwd, cfg.bundle) : undefined,
			include: Array.isArray(cfg.include) ? cfg.include.filter((p) => typeof p === 'string') : undefined,
			bundleCheck: typeof cfg.bundleCheck === 'boolean' ? cfg.bundleCheck : undefined,
		}
	} catch {
		return {}
	}
}

// 常见 screeps 项目布局的 bundle 候选 (screeps-typescript-starter 等)
const DEFAULT_BUNDLE_CANDIDATES = ['dist/main.js', 'dist/main.bundle.js', 'main.js']

function findDefaultBundle(cwd) {
	for (const rel of DEFAULT_BUNDLE_CANDIDATES) {
		const p = resolve(cwd, rel)
		if (existsSync(p)) return p
	}
	return null
}

// ── 运行 ──────────────────────────────────────────────────

function resolveBundle(opts, simCase, cfg) {
	if (opts.bundle) return opts.bundle
	if (simCase.sim.bundle) return resolve(dirname(simCase.file), simCase.sim.bundle)
	if (cfg.bundle) return cfg.bundle
	return findDefaultBundle(process.cwd())
}

async function runCase(bin, simCase, opts, cfg = {}) {
	// 静态预检: 可判定的错误在引擎启动前指出 (编译器层)
	const { errors, warnings } = checkScenario(toWireScenario(simCase.sim))
	if (errors.length) {
		return {
			simCase,
			ok: false,
			reasons: errors.map((e) => `检查 ${e.path}: ${e.message}`),
			warnings,
		}
	}
	const bundle = resolveBundle(opts, simCase, cfg)
	if (!bundle) {
		return {
			simCase,
			ok: false,
			reasons: [
				'未找到 bundle: 用 --bundle 指定, 或用例内 bundle 字段, ' +
					'或在 package.json 配 "screeps-sim": {"bundle": "dist/main.js"}',
			],
			warnings,
		}
	}
	const args = ['--scenario', '-', '--bundle', bundle, '--json-report', '-']
	if (opts.ticks) args.push('--ticks', String(opts.ticks))
	if (opts.verbose) args.push('--verbose')
	const stdinText = JSON.stringify(toWireScenario(simCase.sim))
	const started = performance.now()
	const r = opts.jobs > 1
		? await runBinaryAsync(bin, args, stdinText)
		: runBinaryCapture(bin, args, stdinText)
	const seconds = (performance.now() - started) / 1000

	let report = null
	try {
		report = JSON.parse(r.stdout.trim() || 'null')
	} catch {
		// 引擎崩溃 / 输出被污染
	}
	if (!report || typeof report !== 'object') {
		return {
			simCase,
			ok: false,
			reasons: ['引擎未返回有效 JSON 报告 (可能是场景校验失败或进程崩溃)'],
			stderr: r.stderr,
			seconds,
		}
	}
	const failedAsserts = (report.asserts ?? []).filter((a) => !a.passed)
	const ok = !report.fatalError && failedAsserts.length === 0
	const reasons = []
	for (const a of failedAsserts) reasons.push(`断言 ${a.kind}: ${a.detail}`)
	if (report.fatalError) reasons.push('模拟因致命运行时错误提前终止')
	return { simCase, ok, reasons, report, stderr: r.stderr, seconds, warnings }
}

// ── 输出 ──────────────────────────────────────────────────

const TTY = process.stdout.isTTY
const c = {
	green: (s) => (TTY ? `\x1b[32m${s}\x1b[0m` : s),
	red: (s) => (TTY ? `\x1b[31m${s}\x1b[0m` : s),
	yellow: (s) => (TTY ? `\x1b[33m${s}\x1b[0m` : s),
	dim: (s) => (TTY ? `\x1b[2m${s}\x1b[0m` : s),
	bold: (s) => (TTY ? `\x1b[1m${s}\x1b[0m` : s),
}

function fmtCaseLine(r) {
	const mark = r.ok ? c.green('✓') : c.red('✗')
	const head = `  ${mark} ${r.simCase.sim.name}`
	if (!r.report) return head
	const bits = [
		`${r.report.ticks} ticks`,
		`${r.report.seconds.toFixed(1)}s`,
		`${Math.round(r.report.ticksPerSecond)} t/s`,
	]
	if (r.report.rclTimeline && Object.keys(r.report.rclTimeline).length) {
		const tl = Object.entries(r.report.rclTimeline)
			.sort((a, b) => Number(a[0]) - Number(b[0]))
			.map(([lv, t]) => `L${lv}@${t}`)
			.join(' ')
		bits.push(`RCL ${tl}`)
	}
	if (typeof r.report.cpu?.p95 === 'number') bits.push(`cpu p95 ${r.report.cpu.p95.toFixed(1)}ms`)
	return `${head}  ${c.dim(bits.join('  ·  '))}`
}

function printFailureDetail(r, verbose) {
	for (const reason of r.reasons) console.log(`      ${c.red('·')} ${reason}`)
	if (r.report && r.report.totalErrors > 0) {
		console.log(`      ${c.red('·')} bot 错误 tick 数: ${r.report.totalErrors}`)
	}
	if ((verbose || !r.report) && r.stderr) {
		const lines = r.stderr.trimEnd().split('\n')
		const tail = lines.slice(-15)
		for (const line of tail) console.log(`        ${c.dim(line)}`)
		if (lines.length > tail.length) console.log(`        ${c.dim(`… (省略前 ${lines.length - tail.length} 行)`)}`)
	}
}

// ── 主入口 ────────────────────────────────────────────────

export async function runTests(opts) {
	const cwd = process.cwd()
	const cfg = readProjectConfig(cwd)
	const patterns = opts.patterns.length ? opts.patterns : (cfg.include ?? ['.'])
	const bundleOpt = opts.bundle ?? cfg.bundle
	const files = discoverSimFiles(patterns, cwd)
	if (!files.length) {
		throw new Error(
			'未发现 .sim.ts 用例文件。用法: screeps-sim test [文件|目录|通配符]  ' +
				'(无参数时扫描当前目录, 跳过 node_modules/target/dist 等)'
		)
	}

	const allCases = []
	for (const file of files) {
		const cases = await loadCases(file)
		if (!cases.length) {
			console.error(`${c.red('✗')} ${relative(cwd, file)} 未导出任何用例 (需要 default 导出或具名导出场景对象)`)
			continue
		}
		allCases.push(...cases.map((sc) => ({ ...sc, file })))
	}
	if (!allCases.length) throw new Error('所有文件均未导出用例')

	if (opts.list) {
		for (const { file, sim, exportName } of allCases) {
			console.log(`${relative(cwd, file)}  →  ${sim.name} ${c.dim(`(${exportName})`)}`)
		}
		console.log(`\n共 ${allCases.length} 个用例`)
		return
	}

	const bin = ensureBinary()
	// bundle 静态预检: 每个 bundle 检查一次, 警告打印一次, 错误挂到使用它的用例上
	// --no-bundle-check 或 package.json "screeps-sim".bundleCheck = false: 预检错误降级为警告, 不再判用例失败
	// (误报时的逃生口; 真炸的调用照样会在运行时以 noErrors 断言暴露)。
	const bundleCheck = opts.bundleCheck !== false && cfg.bundleCheck !== false
	const { perBundle, perCase } = preflightBundles(allCases, { ...opts, bundle: bundleOpt }, cfg)
	if (!bundleCheck) perCase.clear()
	for (const [bundle, diag] of perBundle) {
		const shown = bundleCheck ? diag?.warnings ?? [] : [...(diag?.errors ?? []), ...(diag?.warnings ?? [])]
		if (!shown.length) continue
		console.log(c.yellow(`⚠ bundle 预检 ${relative(cwd, bundle) || bundle}${bundleCheck ? '' : ' (已关闭, 仅提示)'}:`))
		for (const w of shown.slice(0, 10)) console.log(c.dim(`    L${w.line}  ${w.message}`))
	}
	const totalStart = performance.now()
	let passed = 0
	const failures = []

	console.log(
		c.bold(`screeps-sim test  ·  ${allCases.length} 个用例`) +
			c.dim(`  ·  bundle: ${bundleOpt ?? '自动发现'}`)
	)
	let skipped = 0
	for (const simCase of allCases) {
		if (simCase.sim.skip) skipped++
	}
	// CPU 敏感断言与并行互斥提示 (墙钟口径会被争抢污染)
	if (opts.jobs > 1) {
		const cpuSensitive = allCases.filter((cse) =>
			(cse.sim.asserts ?? []).some((a) => a.kind === 'cpuAvgBelow' || a.kind === 'cpuP95Below')
		)
		if (cpuSensitive.length) {
			console.log(
				c.yellow(`⚠ --jobs ${opts.jobs} 且 ${cpuSensitive.length} 个用例含 cpu*Below 断言: 并行争抢会使墙钟口径偏大 (建议 -j 1)`)
			)
		}
	}
	const activeCases = allCases.filter((cse) => !cse.sim.skip)
	const results = new Map()
	const runOpts = { ...opts, bundle: bundleOpt }
	if (opts.jobs <= 1) {
		for (const simCase of activeCases) {
			results.set(simCase, await runCase(bin, simCase, runOpts, cfg))
		}
	} else {
		// 并行池: 每用例独立引擎进程, 结果按原序渲染
		let idx = 0
		const worker = async () => {
			while (idx < activeCases.length) {
				const my = idx++
				results.set(activeCases[my], await runCase(bin, activeCases[my], runOpts, cfg))
			}
		}
		await Promise.all(Array.from({ length: Math.min(opts.jobs, activeCases.length) }, worker))
	}

	// 渲染 (保持声明顺序与文件分组)
	let lastFile = null
	for (const simCase of allCases) {
		if (simCase.file !== lastFile) {
			lastFile = simCase.file
			console.log(`\n${c.bold(relative(cwd, simCase.file))}`)
		}
		if (simCase.sim.skip) {
			console.log(`  ${c.dim('− ' + simCase.sim.name + '  (skip)')}`)
			continue
		}
		const bundleDiag = perCase.get(simCase)
		let r = results.get(simCase)
		if (bundleDiag?.errors.length) {
			r = {
				simCase,
				ok: false,
				reasons: bundleDiag.errors.map((e) => `bundle L${e.line}: ${e.message}`),
				warnings: r?.warnings,
			}
		}
		console.log(fmtCaseLine(r))
		for (const w of r.warnings ?? []) {
			console.log(`      ${c.dim('⚠ ' + w.path + ': ' + w.message)}`)
		}
		if (r.ok) {
			passed++
		} else {
			failures.push(r)
			printFailureDetail(r, opts.verbose)
		}
	}

	const totalSeconds = (performance.now() - totalStart) / 1000
	const ran = allCases.length - skipped
	console.log(
		`\n${failures.length ? c.red('存在失败') : c.green('全部通过')}  ·  ` +
			`${passed}/${ran} 通过` +
			(skipped ? `${c.dim(`, ${skipped} 跳过`)}` : '') +
			`  ·  总用时 ${totalSeconds.toFixed(1)}s`
	)
	if (failures.length) process.exit(1)
}

/// 对所有用例将用到的 bundle 做一次静态预检 (按路径去重)。
/// 返回 { perBundle: Map<bundlePath, {errors, warnings}>, perCase: Map<case, {errors, warnings}> }
function preflightBundles(allCases, opts, cfg) {
	const perBundle = new Map()
	for (const simCase of allCases) {
		const bundle = resolveBundle(opts, simCase, cfg)
		if (!bundle) continue // 缺 bundle 由用例自身的失败路径报告
		if (!perBundle.has(bundle)) {
			try {
				perBundle.set(bundle, checkBundle(readFileSync(bundle, 'utf8'), { filename: bundle }))
			} catch {
				perBundle.set(bundle, null) // 读取失败由引擎/用例路径报错
			}
		}
	}
	const perCase = new Map()
	for (const [bundle, diag] of perBundle) {
		if (!diag) continue
		for (const simCase of allCases) {
			if (resolveBundle(opts, simCase, cfg) === bundle) perCase.set(simCase, diag)
		}
	}
	return { perBundle, perCase }
}
/// 错误 → 退出码 1; --strict 时警告也计入失败。
/// `screeps-sim check`: 静态检查 (不跑模拟, 不需要引擎二进制)。
/// 两种模式: `check [patterns]` 检查场景定义 (跳过的用例也会被检查);
///           `check --bundle <path>` 只检查 bot bundle。
export async function runChecks(opts) {
	const cwd = process.cwd()

	if (opts.bundle) {
		const source = readFileSync(opts.bundle, 'utf8')
		const { errors, warnings } = checkBundle(source, { filename: opts.bundle })
		for (const e of errors) console.log(`  ${c.red('✗')} L${e.line}  ${e.message}`)
		for (const w of warnings) console.log(`  ${c.yellow('⚠')} L${w.line}  ${c.dim(w.message)}`)
		if (!errors.length && !warnings.length) console.log(c.green(`✓ bundle 通过静态检查`))
		const failed = errors.length > 0 || (opts.strict && warnings.length > 0)
		console.log(
			`\n${failed ? c.red('检查未通过') : c.green('检查通过')}  ·  错误 ${errors.length}  ·  警告 ${warnings.length}` +
				(opts.strict ? c.dim('  (--strict: 警告计为失败)') : '')
		)
		if (failed) process.exit(1)
		return
	}

	const cfg = readProjectConfig(cwd)
	const patterns = opts.patterns.length ? opts.patterns : (cfg.include ?? ['.'])
	const files = discoverSimFiles(patterns, cwd)
	if (!files.length) throw new Error('未发现 .sim.ts 用例文件 (用法: screeps-sim check [文件|目录|通配符])')

	let totalErrors = 0
	let totalWarnings = 0
	let lastFile = null
	let anyPrinted = false

	for (const file of files) {
		const cases = await loadCases(file)
		for (const simCase of cases) {
			const { errors, warnings } = checkScenario(toWireScenario(simCase.sim))
			totalErrors += errors.length
			totalWarnings += warnings.length
			if (!errors.length && !warnings.length) continue
			if (simCase.file !== lastFile) {
				lastFile = simCase.file
				console.log(`\n${c.bold(relative(cwd, simCase.file))}`)
			}
			anyPrinted = true
			const head = errors.length ? c.red(`✗ ${simCase.sim.name}`) : c.bold(`${simCase.sim.name}`)
			console.log(`  ${head}`)
			for (const e of errors) console.log(`    ${c.red('✗')} ${e.path}: ${e.message}`)
			for (const w of warnings) console.log(`    ${c.yellow('⚠')} ${c.dim(`${w.path}: ${w.message}`)}`)
		}
	}

	if (!anyPrinted) console.log(c.green(`✓ 全部 ${files.length} 个文件通过静态检查, 无警告`))
	const failed = totalErrors > 0 || (opts.strict && totalWarnings > 0)
	console.log(
		`\n${failed ? c.red('检查未通过') : c.green('检查通过')}  ·  错误 ${totalErrors}  ·  警告 ${totalWarnings}` +
			(opts.strict ? c.dim('  (--strict: 警告计为失败)') : '')
	)
	if (failed) process.exit(1)
}
