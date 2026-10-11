// Rust 二进制定位 / 构建 / 透传。
// 解析顺序: SCREEPS_SIM_BIN 环境变量 → <包根>/target/release/screeps-sim → 自动 cargo build。
import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

export const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), '..')

function exeName() {
	return process.platform === 'win32' ? 'screeps-sim.exe' : 'screeps-sim'
}

export function candidateBinaryPaths() {
	const paths = []
	if (process.env.SCREEPS_SIM_BIN) paths.push(process.env.SCREEPS_SIM_BIN)
	paths.push(join(pkgRoot, 'target', 'release', exeName()))
	return paths
}

/// cargo build --release (在包根; 首次使用或源码更新后)
export function buildBinary() {
	console.error('▸ cargo build --release (构建模拟器引擎, 首次约 1 分钟)…')
	const r = spawnSync('cargo', ['build', '--release'], { cwd: pkgRoot, stdio: 'inherit' })
	if (r.error) {
		throw new Error(`无法调用 cargo (${r.error.message})。请安装 Rust 工具链, 或设置 SCREEPS_SIM_BIN 指向已构建的二进制。`)
	}
	if (r.status !== 0) {
		throw new Error(`cargo build 失败 (退出码 ${r.status})`)
	}
}

/// 确保二进制存在, 缺失时自动构建
export function ensureBinary({ build = true } = {}) {
	for (const p of candidateBinaryPaths()) {
		if (existsSync(p)) return p
	}
	if (build) {
		buildBinary()
		for (const p of candidateBinaryPaths()) {
			if (existsSync(p)) return p
		}
	}
	throw new Error(
		'找不到 screeps-sim 二进制。请运行 `npm run build` (需要 Rust 工具链), ' +
			'或设置环境变量 SCREEPS_SIM_BIN 指向已有二进制。'
	)
}

/// 其余子命令原样透传给 Rust 二进制
export function forwardToBinary(args) {
	const bin = ensureBinary()
	const r = spawnSync(bin, args, { stdio: 'inherit' })
	process.exit(r.status ?? 1)
}

/// 捕获式运行: 场景走 stdin, JSON 报告从 stdout 收 (引擎的管道协议)
export function runBinaryCapture(bin, args, stdinText) {
	return spawnSync(bin, args, {
		input: stdinText,
		encoding: 'utf8',
		maxBuffer: 256 * 1024 * 1024,
	})
}
