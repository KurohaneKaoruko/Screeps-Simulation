// 安装冒烟测试 (发布就绪的最后一道关): 把主包真的 npm pack → 装进临时项目 → 跑用例。
//
//   node scripts/test-install.mjs [--keep] [--skip-platform]
//
// 两轮:
//   A. 源码/无平台包: 只装主包 tgz, 用 SCREEPS_SIM_BIN 指向本仓库二进制 → 验证 CLI + runner + 类型文件可用
//   B. 平台包: 再把当前平台的 `@screeps-sim/<key>` tgz 装进同一项目, 且**不设** SCREEPS_SIM_BIN
//      → 验证 optionalDependencies 的解析路径 (注册表安装时的真实形态)
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { detectPlatform } from '../runner/platform.mjs'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const argv = process.argv.slice(2)
const keep = argv.includes('--keep')
const skipPlatform = argv.includes('--skip-platform')
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
	const r = spawnSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', args, {
		cwd,
		encoding: 'utf8',
		env: { ...process.env, ...extraEnv },
	})
	return { ...r, out: `${r.stdout || ''}${r.stderr || ''}` }
}

const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const plat = detectPlatform()
const tmp = mkdtempSync(join(tmpdir(), 'screeps-sim-install-'))
console.log(`安装冒烟: ${pkg.name}@${pkg.version} → ${tmp}`)

try {
	// ── 打包主包 ──────────────────────────────────────────
	console.log('\n1. npm pack')
	const packed = npm(['pack', '--pack-destination', tmp], root)
	// npm 会把 prepack 脚本输出混进 stdout/stderr, 直接扫描 tmp 目录找 tgz 更可靠
	let tgz = null
	try {
		const files = readdirSync(tmp)
		tgz = files.find(f => f.startsWith(`${pkg.name.replace(/[/@]/g, '')}-`) && f.endsWith('.tgz'))
	} catch {}
	if (!step(packed.status === 0 && !!tgz && existsSync(join(tmp, tgz)), '主包打包成功', packed.out)) process.exit(1)
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

	// ── A. 只装主包 (SCREEPS_SIM_BIN 指本仓库二进制) ──────
	console.log('\n2. 只装主包 (源码安装形态: SCREEPS_SIM_BIN)')
	const install = npm(
		['install', '--no-audit', '--no-fund', '--omit=optional', join(tmp, tgz)],
		proj,
		{ SCREEPS_SIM_BIN: repoBin }
	)
	if (!step(install.status === 0, 'npm install 主包', install.out)) process.exit(1)
	const installed = join(proj, 'node_modules', pkg.name)
	step(existsSync(join(installed, 'index.mjs')) && existsSync(join(installed, 'index.d.ts')), '主入口与类型文件已安装')
	step(
		existsSync(join(proj, 'node_modules', '.bin', 'screeps-sim')) ||
			existsSync(join(proj, 'node_modules', '.bin', 'screeps-sim.cmd')),
		'bin 链接已建立 (npx screeps-sim)'
	)

	const binPath = join(proj, 'node_modules', '.bin', process.platform === 'win32' ? 'screeps-sim.cmd' : 'screeps-sim')
	const runCli = (args, env = {}) =>
		spawnSync(process.execPath, [binPath, ...args], { cwd: proj, encoding: 'utf8', env: { ...process.env, ...env } })

	const ver = runCli(['version'], { SCREEPS_SIM_BIN: repoBin })
	step(ver.status === 0 && ver.stdout.includes(pkg.version), 'screeps-sim version', `${ver.stdout}${ver.stderr}`)

	const doc = runCli(['doctor'], { SCREEPS_SIM_BIN: repoBin })
	step(doc.status === 0 && doc.stdout.includes('SCREEPS_SIM_BIN'), 'screeps-sim doctor (命中环境变量)', doc.stdout)

	const run1 = runCli(['test'], { SCREEPS_SIM_BIN: repoBin })
	step(run1.status === 0, 'screeps-sim test (含 .sim.ts + 零依赖用例 + bot bundle)', `${run1.stdout}\n${run1.stderr}`)

	// 缺二进制时的报错必须可执行 (含平台包名), 而不是裸栈
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

	// ── B. 装平台包, 不给 SCREEPS_SIM_BIN ─────────────────
	if (skipPlatform || !plat.supported) {
		console.log(`\n3. 跳过平台包验证${plat.supported ? ' (--skip-platform)' : ` (当前平台 ${plat.platform}/${plat.arch} 无预编译包)`}`)
	} else {
		console.log(`\n3. 平台包安装 (${plat.packageName})`)
		let platformTgz = join(root, 'dist', 'platform', `${plat.key}.tgz`)
		const manifest = join(root, 'dist', 'platform', `${plat.key}.json`)
		if (existsSync(manifest)) {
			const m = JSON.parse(readFileSync(manifest, 'utf8'))
			platformTgz = join(root, m.tgz)
		} else {
			const built = spawnSync(
				process.execPath,
				[join(root, 'scripts', 'build-platform-pkg.mjs'), '--skip-build'],
				{ cwd: root, encoding: 'utf8' }
			)
			step(built.status === 0, `构建平台包 ${plat.key}`, `${built.stdout}${built.stderr}`)
			if (built.status === 0) {
				const m = JSON.parse(readFileSync(join(root, 'dist', 'platform', `${plat.key}.json`), 'utf8'))
				platformTgz = join(root, m.tgz)
			}
		}
		step(existsSync(platformTgz), `平台包 tgz 就绪 (${platformTgz})`)

		if (existsSync(platformTgz)) {
			// 直接装成顶层依赖 (模拟 npm 把 optionalDependencies 提升到项目 node_modules 的形态)
			const pinstall = npm(['install', '--no-audit', '--no-fund', platformTgz], proj, {})
			step(pinstall.status === 0, 'npm install 平台包', pinstall.out)

			const doc2 = runCli(['doctor'], { SCREEPS_SIM_BIN: '' })
			step(
				doc2.status === 0 && doc2.stdout.includes(plat.packageName) && !doc2.stdout.includes('环境变量'),
				`doctor 命中平台包 ${plat.packageName} (未设 SCREEPS_SIM_BIN)`,
				doc2.stdout
			)

			const run2 = runCli(['test'], { SCREEPS_SIM_BIN: '' })
			step(run2.status === 0, 'screeps-sim test (平台包二进制)', `${run2.stdout}\n${run2.stderr}`)
		}
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
