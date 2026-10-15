// Rust 二进制定位 / 构建 / 透传。
// 解析顺序: SCREEPS_SIM_BIN 环境变量 → 源码树 target/release → 平台包 (@screeps-sim/<key>)
//           → 自动 cargo build (仅源码安装) → 报错并给出可执行的排障步骤。
import { spawnSync, spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { existsSync, statSync, accessSync, constants as fsConstants } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { detectPlatform, SCOPE } from './platform.mjs'

export const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), '..')

export function exeName() {
	return process.platform === 'win32' ? 'screeps-sim.exe' : 'screeps-sim'
}

/// 本地开发/源码安装: 是否带 Rust 源码 (决定能不能 cargo build)
export function hasRustSources() {
	return existsSync(join(pkgRoot, 'Cargo.toml'))
}

/// 平台包里的二进制: node_modules/@screeps-sim/<key>/bin/screeps-sim[.exe]
/// (npm 把 optionalDependencies 提升到项目 node_modules, 从包根 require 即可向上找到;
///  嵌套安装时也在包内, 同样命中)
export function platformPackageBinary(plat = detectPlatform()) {
	if (!plat.supported) return null
	const binPath = join('bin', exeName())
	// 平台包可能只暴露 exports 而不放行 package.json, 两种入口都试
	const bases = [join(pkgRoot, 'package.json'), join(process.cwd(), 'noop.cjs')]
	for (const base of bases) {
		let req
		try {
			req = createRequire(base)
		} catch {
			continue
		}
		for (const sub of ['package.json', binPath, `./${binPath}`]) {
			let file
			try {
				file = req.resolve(`${plat.packageName}/${sub}`)
			} catch {
				continue
			}
			const target = sub === 'package.json' ? join(dirname(file), binPath) : file
			if (isRunnable(target)) return target
		}
	}
	return null
}

function isRunnable(p) {
	if (!existsSync(p)) return false
	try {
		accessSync(p, fsConstants.X_OK)
		return statSync(p).isFile()
	} catch {
		return false
	}
}

/// 全部候选 (按优先级): 返回 { path, source } 便于 doctor 展示来源
export function candidateBinaries() {
	const out = []
	if (process.env.SCREEPS_SIM_BIN) out.push({ path: process.env.SCREEPS_SIM_BIN, source: '环境变量 SCREEPS_SIM_BIN' })
	const local = join(pkgRoot, 'target', 'release', exeName())
	// 源码树里已有构建产物 → 视为开发者 checkout, 本地构建优先于平台包 (改代码即时生效)
	if (isRunnable(local)) out.push({ path: local, source: '源码构建 target/release' })
	const plat = detectPlatform()
	const pkgBin = platformPackageBinary(plat)
	if (pkgBin) out.push({ path: pkgBin, source: `平台包 ${plat.packageName}` })
	return out
}

/// 兼容旧调用: 只取候选路径
export function candidateBinaryPaths() {
	return candidateBinaries().map((c) => c.path)
}

/// 找到可用的引擎二进制 (不构建); 没找到返回 null
export function findBinary() {
	return candidateBinaries().find((c) => isRunnable(c.path)) ?? null
}

/// SCREEPS_SIM_BIN 是显式指定: 指错了就是配置错误, 立刻报错而不是悄悄回退 (省掉几十分钟的误编译)
export function checkExplicitBin() {
	const p = process.env.SCREEPS_SIM_BIN
	if (p && !isRunnable(p)) {
		throw installError(`SCREEPS_SIM_BIN 指向的二进制不可用: ${p}\n  该文件不存在或没有执行权限。`)
	}
}

/// 二进制的完整信息 (纯查询, 不启动进程): 编程 API / doctor / 排障共用
export function binaryInfo() {
	const plat = detectPlatform()
	const explicit = process.env.SCREEPS_SIM_BIN
	const found = findBinary()
	// 显式指定但不可用: 立刻可见, 不让 doctor 说"没找到"而掩盖真实原因
	if (explicit && !isRunnable(explicit) && found?.source !== '环境变量 SCREEPS_SIM_BIN') {
		return {
			ok: false,
			path: null,
			source: null,
			platform: plat,
			platformPackageInstalled: !!platformPackageBinary(plat),
			sourceTree: hasRustSources(),
			invalidEnvPath: explicit,
		}
	}
	return {
		ok: !!found,
		path: found?.path ?? null,
		source: found?.source ?? null,
		platform: plat,
		/** 平台包是否已安装 (未装则需要 Rust 工具链或手动指定) */
		platformPackageInstalled: !!platformPackageBinary(plat),
		/** 是否是源码安装 (带 Cargo.toml, 可本地编译) */
		sourceTree: hasRustSources(),
	}
}

/// cargo build --release (在包根; 首次使用或源码更新后)
export function buildBinary() {
	if (!hasRustSources()) {
		throw installError(`包内没有 Rust 源码 (${join(pkgRoot, 'Cargo.toml')} 不存在), 无法编译引擎。`)
	}
	console.error('▸ cargo build --release (构建模拟器引擎, 首次约 1 分钟)…')
	const r = spawnSync('cargo', ['build', '--release'], { cwd: pkgRoot, stdio: 'inherit' })
	if (r.error) {
		throw installError(
			`无法调用 cargo (${r.error.message})。请安装 Rust 工具链 (https://rustup.rs), ` +
				`或安装平台包 ${SCOPE}/<platform>、或设置 SCREEPS_SIM_BIN 指向已构建的二进制。`
		)
	}
	if (r.status !== 0) {
		throw new Error(`cargo build 失败 (退出码 ${r.status})`)
	}
}

/// 确保二进制存在, 缺失时自动构建 (仅源码安装可用)
export function ensureBinary({ build = true } = {}) {
	checkExplicitBin()
	const found = findBinary()
	if (found) return found.path
	if (build && hasRustSources()) {
		buildBinary()
		const again = findBinary()
		if (again) return again.path
	}
	throw installError(missingBinaryMessage())
}

/// 环境/安装类错误: 标记后由 CLI 只打消息 (不吐调用栈 —— 栈对使用者没意义, 只会淹没提示)
function installError(message) {
	const err = new Error(message)
	err.code = 'ERR_SIM_NO_BINARY'
	return err
}

/// 找不到二进制时的可执行提示 (doctor / 报错共用)
export function missingBinaryMessage() {
	const plat = detectPlatform()
	const tried = [...candidateBinaries().map((c) => `  · ${c.path}  (${c.source})`), `  · cargo build --release  (${pkgRoot})`]
	const lines = [`找不到 screeps-sim 引擎二进制。已尝试:`, ...tried]
	if (plat.supported) {
		lines.push(`当前平台: ${plat.platform}/${plat.arch}${plat.libc ? ` (${plat.libc})` : ''} → 需要平台包 ${plat.packageName}`)
		lines.push(`解决: ① npm i -D ${plat.packageName}  ② npm i -D screeps-sim (自动安装平台包)  ③ 设置 SCREEPS_SIM_BIN=<二进制路径>`)
	} else {
		lines.push(`当前平台 ${plat.platform}/${plat.arch} 没有官方预编译包: 请自行 cargo build --release, 或设置 SCREEPS_SIM_BIN 指向已有二进制。`)
	}
	lines.push(`诊断详情: screeps-sim doctor`)
	return lines.join('\n')
}

/// 其余子命令原样透传给 Rust 二进制
export function forwardToBinary(args) {
	const bin = ensureBinary()
	const r = spawnSync(bin, args, { stdio: 'inherit' })
	process.exit(r.status ?? 1)
}

/// 捕获式运行 (同步): 场景走 stdin, JSON 报告从 stdout 收 (引擎的管道协议)
export function runBinaryCapture(bin, args, stdinText) {
	return spawnSync(bin, args, {
		input: stdinText,
		encoding: 'utf8',
		maxBuffer: 256 * 1024 * 1024,
	})
}

/// 捕获式运行 (异步): 并行模式用, 不阻塞事件循环
export function runBinaryAsync(bin, args, stdinText) {
	return new Promise((resolve) => {
		const child = spawn(bin, args, { stdio: ['pipe', 'pipe', 'pipe'] })
		let stdout = ''
		let stderr = ''
		child.stdout.on('data', (c) => (stdout += c))
		child.stderr.on('data', (c) => (stderr += c))
		child.on('error', (e) => resolve({ status: 1, stdout, stderr: String(e), error: e }))
		child.on('close', (status) => resolve({ status, stdout, stderr }))
		child.stdin.on('error', () => {}) // 引擎提前退出时忽略 EPIPE
		child.stdin.end(stdinText)
	})
}
