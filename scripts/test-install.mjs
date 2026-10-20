// 安装冒烟测试 (发布就绪的最后一道关): 把主包真的 npm pack → 装进临时项目 → 跑用例。
//
//   node scripts/test-install.mjs [--keep] [--skip-download]
//
// 两轮:
//   A. 源码形态: 用 SCREEPS_SIM_BIN 指向本仓库二进制 → 验证 CLI + runner + 类型文件可用
//   B. 安装形态: **不设** SCREEPS_SIM_BIN → 验证自动从 GitHub Release 下载引擎 (SHA256 校验)
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { detectPlatform } from '../runner/platform.mjs'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const argv = process.argv.slice(2)
const keep = argv.includes('--keep')
const skipDownload = argv.includes('--skip-download')
const exeName = process.platform === 'win32' ? 'screeps-sim.exe' : 'screeps-sim'
const repoBin = join(root, 'target', 'release', exeName)

let failed = 0
const step = (ok, label, detail = '') => {
	if (ok) console.log(`  ✓ ${label}`)
	else {
		failed++
		console.error(`  ✗ ${label}${detail ? `\n      ${String(detail).trim().split('\n').join('\n      ')}` : ''}`)
	}
	return ok
}

function npm(args, cwd, extraEnv = {}) {
	const cmd = process.platform === 'win32' ? 'npm.cmd' : 'npm'
	const r = spawnSync(cmd, args, {
		cwd,
		encoding: 'utf8',
		env: { ...process.env, ...extraEnv },
		shell: true,
	})
	return { ...r, out: `${r.stdout || ''}${r.stderr || ''}` }
}

const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const plat = detectPlatform()
const tmp = mkdtempSync(join(tmpdir(), 'screeps-sim-install-'))
console.log(`安装冒烟: ${pkg.name}@${pkg.version} → ${tmp}`)

try {
	// ── 打包主包
	console.log('\n1. npm pack')
	const packed = npm(['pack', '--pack-destination', tmp], root)
	const tgzName = (packed.stdout || '').trim().split('\n').filter(Boolean).pop()?.trim()
	const tgz = tgzName ? join(tmp, tgzName) : null
	if (!step(packed.status === 0 && tgz && existsSync(tgz), 'npm pack 主包', packed.out)) {
		console.error('=== npm pack 完整输出 ===')
		console.error(packed.out)
		console.error('============================')
		process.exit(1)
	}
	console.log(`  tgz: ${tgz}`)

	// ── 临时 bot 项目 ─────────────────────────────────────
	const proj = join(tmp, 'proj')
	mkdirSync(join(proj, 'sim'), { recursive: true })
	writeFileSync(
		join(proj, 'package.json'),
		JSON.stringify(
			{
				name: 'consumer-bot',
				version: '0.0.0',
				private: true,
				type: 'module',
				scripts: { sim: 'screeps-sim test' },
				// 项目级配置: bundle 相对项目根, 替代每次 --bundle
				'screeps-sim': { bundle: 'bot.js' },
			},
			null,
			2
		)
	)
	// 用例 ①: 正规写法 (import defineSim, 验证类型文件 + 主入口) ②: 零依赖纯对象导出
	writeFileSync(
		join(proj, 'sim', 'smoke.sim.ts'),
		`import { defineSim, binaryInfo } from '${pkg.name}'

// ① bundle 走项目 package.json 的 "screeps-sim".bundle
const case1 = defineSim({
	name: 'install-smoke',
	ticks: 30,
	seed: 7,
	rooms: [{
		name: 'W1N1',
		controller: { level: 1, pos: [25, 25] },
		sources: { positions: [[20, 20]] },
		structures: [{ type: 'spawn', pos: [24, 24], energy: 300 }],
		creeps: [{ name: 'W1', body: ['work', 'carry', 'move'], pos: [23, 24] }],
	}],
	asserts: [{ kind: 'creepCountAtLeast', value: 1 }, { kind: 'noErrors' }],
})

export default case1

// ② 零依赖写法 (不 import 任何东西, 纯对象导出也要被识别) + 用例内 bundle 字段
export const zeroDeps = {
	name: 'install-smoke-zero-deps',
	ticks: 10,
	bundle: '../bot.js',
	rooms: [{ name: 'W2N2', controller: { level: 1 }, sources: { count: 2 } }],
	asserts: [{ kind: 'noErrors' }],
}

// 编程 API 可用 (类型声明 + 主入口导出)
if (typeof binaryInfo().platform.platform !== 'string') throw new Error('binaryInfo() 异常')
`
	)
	// 陪跑 bundle: 用包里自带的 fixtures (顺带验证 files 白名单没漏 examples/)
	const fixture = join(root, 'examples', 'fixtures', 'min-bot.js')
	step(existsSync(fixture), '陪跑 bundle 存在于 examples/fixtures/min-bot.js')
	writeFileSync(join(proj, 'bot.js'), readFileSync(fixture, 'utf8'))

	// ── A. 源码形态 (SCREEPS_SIM_BIN 指本仓库二进制) ──────
	console.log('\n2. 安装主包 (源码形态: SCREEPS_SIM_BIN 指向本仓库二进制)')
	const install = npm(['install', '--ignore-scripts', '--no-audit', '--no-fund', tgz], proj, {})
	step(install.status === 0, 'npm install 主包 tgz', install.out)

	const binName = process.platform === 'win32' ? 'screeps-sim.cmd' : 'screeps-sim'
	const binPath = join(proj, 'node_modules', '.bin', binName)
	// Windows: .bin 是 CMD 垫片, shell:true 时第一参数必须是字符串 (cmd.exe /c "命令 参数...")
	// 非 Windows: 用 node 直接跑入口 .mjs
	const runCli = (args, env = {}) => {
		if (process.platform === 'win32') {
			const cmd = `"${binPath}" ${args.map(a => `"${a}"`).join(' ')}`
			return spawnSync(cmd, { cwd: proj, encoding: 'utf8', env: { ...process.env, ...env }, shell: true })
		}
		return spawnSync(process.execPath, [binPath, ...args], { cwd: proj, encoding: 'utf8', env: { ...process.env, ...env } })
	}

	const ver = runCli(['version'], { SCREEPS_SIM_BIN: repoBin })
	step(ver.status === 0 && ver.stdout.includes(pkg.version), 'screeps-sim version', `${ver.stdout}${ver.stderr}`)

	const doc = runCli(['doctor'], { SCREEPS_SIM_BIN: repoBin })
	step(doc.status === 0 && doc.stdout.includes('SCREEPS_SIM_BIN'), 'screeps-sim doctor (命中环境变量)', doc.stdout)

	const run1 = runCli(['test'], { SCREEPS_SIM_BIN: repoBin })
	step(run1.status === 0, 'screeps-sim test (含 .sim.ts + 零依赖用例 + bot bundle)', `${run1.stdout}\n${run1.stderr}`)

	// 缺二进制时的报错必须可执行 (指向下载/环境变量), 而不是裸栈
	// SCREEPS_SIM_BIN 指错 → 立刻报配置错误 (不能悄悄回退去编译几分钟)
	const badEnv = runCli(['doctor'], { SCREEPS_SIM_BIN: join(proj, 'no-such-binary') })
	step(
		badEnv.status === 1 && badEnv.stdout.includes('SCREEPS_SIM_BIN 指向的二进制不可用'),
		'SCREEPS_SIM_BIN 指错时 doctor 直指配置错误',
		badEnv.stdout
	)
	const badRun = runCli(['test'], { SCREEPS_SIM_BIN: join(proj, 'no-such-binary') })
	step(
		badRun.status === 1 && !badRun.stderr.includes('at ensureBinary') && badRun.stderr.includes('SCREEPS_SIM_BIN'),
		'SCREEPS_SIM_BIN 指错时报可执行提示, 不吐调用栈',
		`${badRun.stdout}${badRun.stderr}`
	)

	// ── B. 安装形态: 不给 SCREEPS_SIM_BIN → 自动从 GitHub Release 下载 ──
	if (skipDownload || !plat.supported) {
		console.log(`\n3. 跳过 Release 下载验证${plat.supported ? ' (--skip-download)' : ` (当前平台 ${plat.platform}/${plat.arch} 无预编译引擎)`}`)
	} else {
		console.log(`\n3. 安装形态: 无 SCREEPS_SIM_BIN, 自动从 GitHub Release 下载引擎 (${plat.key})`)
		// doctor 是纯查询 (不下载不构建): 无二进制时应报「未找到」且修复提示指向 Release 下载
		const dl = runCli(['doctor'], {
			SCREEPS_SIM_BIN: '',
			XDG_CACHE_HOME: join(tmp, 'cache'),
		})
		step(
			dl.status === 1 && dl.stdout.includes('GitHub Release 下载'),
			'doctor (纯查询) 报未找到且修复提示指向 Release 下载',
			dl.stdout
		)
		const run2 = runCli(['test'], { SCREEPS_SIM_BIN: '', XDG_CACHE_HOME: join(tmp, 'cache') })
		step(run2.status === 0, 'screeps-sim test (自动从 Release 下载引擎后运行)', `${run2.stdout}\n${run2.stderr}`)
	}
} finally {
	if (keep) console.log(`\n(保留临时目录: ${tmp})`)
	else rmSync(tmp, { recursive: true, force: true })
}

if (failed) {
	console.error(`\n安装冒烟失败: ${failed} 项`)
	process.exit(1)
}
console.log('\n安装冒烟全部通过')
