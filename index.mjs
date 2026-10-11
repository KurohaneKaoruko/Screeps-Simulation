// 编程 API: defineSim 标记用例 (供 runner 识别 + TS 类型), runSim 单用例直跑。
import { ensureBinary, runBinaryCapture } from './runner/binary.mjs'
import { toWireScenario } from './runner/wire.mjs'

/// 在 .sim.ts 中声明一个模拟用例 (返回值只做标记, 字段原样保留)
export function defineSim(scenario) {
	return { ...scenario, __simCase: true }
}

/// 直跑单个用例, 返回引擎的结构化报告 (断言结果在 report.asserts)
export function runSim(sim, opts = {}) {
	const bin = ensureBinary()
	const bundle = opts.bundle ?? 'dist/main.js'
	const args = ['--scenario', '-', '--bundle', bundle, '--json-report', '-']
	if (opts.ticks) args.push('--ticks', String(opts.ticks))
	if (opts.verbose) args.push('--verbose')
	const r = runBinaryCapture(bin, args, JSON.stringify(toWireScenario(sim)))
	if (!r.stdout.trim()) {
		throw new Error(`引擎未返回报告 (退出码 ${r.status}):\n${r.stderr}`)
	}
	try {
		return JSON.parse(r.stdout)
	} catch (e) {
		throw new Error(`报告解析失败: ${e}\nstdout: ${r.stdout.slice(0, 2000)}`)
	}
}
