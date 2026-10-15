// doctor: 打印安装/平台/二进制的诊断信息 (装不上、跑不起来时先跑这个)。
// 纯查询: 不构建、不启动模拟, 只看得到什么、为什么没拿到二进制。
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { binaryInfo, missingBinaryMessage, pkgRoot } from './binary.mjs'
import { TARGETS } from './platform.mjs'

function rootPkg() {
	try {
		return JSON.parse(readFileSync(join(pkgRoot, 'package.json'), 'utf8'))
	} catch {
		return {}
	}
}

/// 极简 engines 检查: `>=x.y[.z]` / `^x.y[.z]` 逐段比较, 段数不足按 0 补 (>=22.6 兼容 v22.6.0)
function nodeVersionOk(range) {
	const parse = (v) => {
		const m = /(\d+)\.(\d+)(?:\.(\d+))?/.exec(v)
		return m ? [+m[1], +m[2], m[3] === undefined ? 0 : +m[3]] : null
	}
	const cur = parse(String(process.version).replace(/^v/, ''))
	const want = parse(range ?? '')
	if (!cur || !want) return null
	for (let i = 0; i < 3; i++) {
		if (cur[i] > want[i]) return true
		if (cur[i] < want[i]) return false
	}
	return true
}

function humanSize(bytes) {
	if (!Number.isFinite(bytes)) return '未知'
	return bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${(bytes / 1024).toFixed(0)} KB`
}

/// 引擎二进制自报版本 (二进制太老没有 --version 时静默降级)
function engineVersion(binPath) {
	if (!binPath) return null
	try {
		const r = spawnSync(binPath, ['--version'], { encoding: 'utf8', timeout: 10_000 })
		const out = `${r.stdout || ''}${r.stderr || ''}`.trim()
		return r.status === 0 ? out.split('\n')[0] : null
	} catch {
		return null
	}
}

/// bot 项目侧的配置 (package.json 的 "screeps-sim" 字段 + bundle 是否存在)
function projectInfo(cwd) {
	const info = { cwd, config: false, bundle: null, bundleExists: null, tsSupport: true }
	try {
		const pkg = JSON.parse(readFileSync(join(cwd, 'package.json'), 'utf8'))
		const cfg = pkg['screeps-sim']
		if (cfg && typeof cfg === 'object') {
			info.config = true
			if (typeof cfg.bundle === 'string') {
				info.bundle = resolve(cwd, cfg.bundle)
				info.bundleExists = existsSync(info.bundle)
			}
		}
	} catch {
		/* 无 package.json: 不算问题 */
	}
	info.tsSupport = nodeVersionOk(rootPkg().engines?.node) !== false
	return info
}

/// 结构化诊断结果 (编程 API / 测试用)
export function doctorReport(cwd = process.cwd()) {
	const pkg = rootPkg()
	const info = binaryInfo()
	return {
		package: { name: pkg.name ?? 'screeps-sim', version: pkg.version ?? '?', license: pkg.license ?? null },
		node: { version: process.version, required: pkg.engines?.node ?? null, ok: nodeVersionOk(pkg.engines?.node) },
		platform: {
			...info.platform,
			published: TARGETS.map((t) => t.key),
		},
		binary: {
			ok: info.ok,
			path: info.path,
			source: info.source,
			version: engineVersion(info.path),
			sizeBytes: info.path && existsSync(info.path) ? statSync(info.path).size : null,
			platformPackageInstalled: info.platformPackageInstalled,
			sourceTree: info.sourceTree,
				/** SCREEPS_SIM_BIN 指了但不可用时的原值 */
		invalidEnvPath: info.invalidEnvPath ?? null,
		},
		project: projectInfo(cwd),
		hint: info.invalidEnvPath
			? `SCREEPS_SIM_BIN 指向的二进制不可用: ${info.invalidEnvPath}\n  该文件不存在或没有执行权限。`
			: info.ok
				? null
				: missingBinaryMessage(),
	}
}

/// 人类可读诊断输出; 返回进程退出码 (二进制不可用 → 1)
export function printDoctor(cwd = process.cwd()) {
	const r = doctorReport(cwd)
	const tick = (ok) => (ok === null ? '?' : ok ? '✓' : '✗')
	const out = []
	out.push(`${r.package.name} ${r.package.version}${r.package.license ? `  (${r.package.license})` : ''}`)
	out.push(`  node            ${r.node.version}  (要求 ${r.node.required ?? '未声明'}) ${tick(r.node.ok)}`)
	const p = r.platform
	out.push(
		`  平台            ${p.platform}/${p.arch}${p.libc ? ` (${p.libc})` : ''} ` +
			`${p.supported ? `→ ${p.packageName} ✓` : `→ 无官方预编译包 ✗ (已发布: ${r.platform.published.join(', ')})`}`
	)
	out.push(`  引擎二进制      ${r.binary.ok ? r.binary.path : '未找到 ✗'}`)
	if (r.binary.ok) {
		out.push(`    来源          ${r.binary.source}`)
		out.push(`    引擎版本      ${r.binary.version ?? '未知 (二进制过旧?)'}`)
		out.push(`    大小          ${humanSize(r.binary.sizeBytes)}`)
	}
	out.push(`    平台包        ${r.binary.platformPackageInstalled ? '已安装' : '未安装'}`)
	out.push(`    源码可编译    ${r.binary.sourceTree ? '是 (可 cargo build --release)' : '否 (发布包不含源码)'}`)
	out.push(`  项目            ${r.project.cwd}`)
	out.push(`    screeps-sim 配置  ${r.project.config ? 'package.json 里有' : '无 (用 --bundle 或自动探测)'}`)
	if (r.project.bundle) {
		out.push(`    bundle        ${r.project.bundle}${r.project.bundleExists ? ' ✓' : ' ✗ 不存在'}`)
	}
	out.push(`    .sim.ts 加载  ${r.project.tsSupport ? '✓ (Node 原生 TS)' : '✗ 需 Node ≥ 22.6, 或改用 .sim.mjs'}`)
	if (r.hint) {
		out.push('')
		out.push(r.hint)
	}
	console.log(out.join('\n'))
	return r.binary.ok ? 0 : 1
}
