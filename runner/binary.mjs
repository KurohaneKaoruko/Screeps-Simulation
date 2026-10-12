// Rust 二进制定位 / 构建 / 透传。
// 解析顺序: SCREEPS_SIM_BIN 环境变量 → <包根>/target/release/screeps-sim → 自动 cargo build。
import { spawnSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
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

/// 场景 JSON 写入临时文件并返回路径 (stdin 管道不可用时的降级通道)
function writeScenarioTemp(stdinText) {
	const dir = mkdtempSync(join(tmpdir(), 'screeps-sim-'))
	const p = join(dir, 'scenario.json')
	writeFileSync(p, stdinText)
	return p
}

/// 把参数里的 `--scenario -` / `-s -` 改写为临时文件路径 (其余原样)
function withScenarioFile(args, file) {
	return args.map((a, i) =>
		i > 0 && (args[i - 1] === '--scenario' || args[i - 1] === '-s') && a === '-' ? file : a
	)
}

/// stdin 匿名管道是否被系统拒绝 (部分 Windows 安全软件/策略 hook CreatePipe, EBUSY 231)。
/// 表现: spawnSync/spawn 返回 error.code === 'EBUSY' ("All pipe instances are busy")。
function isStdinPipeBlocked(r) {
	return !!r && (r.error?.code === 'EBUSY' || (r.status === null && r.signal === null && r.error))
}

/// 捕获式运行 (同步): 场景走 stdin, JSON 报告从 stdout 收 (引擎的管道协议)。
/// stdin 管道被系统拒绝时自动降级: 场景写临时文件走 --scenario <file> (语义不变)。
export function runBinaryCapture(bin, args, stdinText) {
	let r = spawnSync(bin, args, {
		input: stdinText,
		encoding: 'utf8',
		maxBuffer: 256 * 1024 * 1024,
	})
	if (isStdinPipeBlocked(r) && stdinText != null) {
		const tmp = writeScenarioTemp(stdinText)
		try {
			r = spawnSync(bin, withScenarioFile(args, tmp), {
				encoding: 'utf8',
				maxBuffer: 256 * 1024 * 1024,
				stdio: ['ignore', 'pipe', 'pipe'], // stdin 不再建管道 (正是被拒的那个)
			})
		} finally {
			rmSync(tmp, { force: true })
			try {
				rmSync(dirname(tmp), { recursive: true, force: true })
			} catch {}
		}
	}
	return r
}

/// 捕获式运行 (异步): 并行模式用, 不阻塞事件循环。
/// stdin 管道被拒绝时同样降级到临时文件 (见 runBinaryCapture)。
export function runBinaryAsync(bin, args, stdinText) {
	return new Promise((resolve) => {
		const runWith = (useFile) => {
			const child = useFile
				? spawn(bin, withScenarioFile(args, useFile), { stdio: ['ignore', 'pipe', 'pipe'] })
				: spawn(bin, args, { stdio: ['pipe', 'pipe', 'pipe'] })
			let stdout = ''
			let stderr = ''
			let tmpDir = null
			if (useFile) tmpDir = dirname(useFile)
			child.stdout.on('data', (c) => (stdout += c))
			child.stderr.on('data', (c) => (stderr += c))
			child.on('error', (e) => {
				if (!useFile && e.code === 'EBUSY' && stdinText != null) {
					// stdin 管道被拒 → 降级临时文件重试一次
					const tmp = writeScenarioTemp(stdinText)
					// 先移除这次错误创建的 child 的监听残留 (尚未产出输出)
					runWith(tmp).then(resolve)
					return
				}
				resolve({ status: 1, stdout, stderr: String(e), error: e })
			})
			child.on('close', (status) => {
				if (tmpDir) {
					try {
						rmSync(tmpDir, { recursive: true, force: true })
					} catch {}
				}
				resolve({ status, stdout, stderr })
			})
			if (!useFile) {
				child.stdin.on('error', () => {}) // 引擎提前退出时忽略 EPIPE
				child.stdin.end(stdinText)
			}
		}
		runWith(null)
	})
}
