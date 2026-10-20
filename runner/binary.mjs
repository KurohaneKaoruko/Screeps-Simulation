// Rust 二进制定位 / 构建 / 下载 / 透传。
// 解析顺序: SCREEPS_SIM_BIN 环境变量 → 源码树 target/release → 包内 vendor/<key>/ →
//           全局缓存 ~/.cache/screeps-sim/<ver>/<key>/ → 从 GitHub Release 自动下载 (带 SHA256 校验)
//           → 自动 cargo build (仅源码安装) → 报错并给出可执行的排障步骤。
import { spawnSync, spawn } from 'node:child_process'
import { chmodSync, copyFileSync, createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, statSync, accessSync, constants as fsConstants } from 'node:fs'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { detectPlatform, targetByKey } from './platform.mjs'

export const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), '..')

/// GitHub 仓库 (Release 分发的唯一来源); fork 后发布需改成自己的仓库
const GITHUB_REPO = 'KurohaneKaoruko/Screeps-Simulation'

export function exeName() {
	return process.platform === 'win32' ? 'screeps-sim.exe' : 'screeps-sim'
}

/// 包随附版本 (package.json): 二进制 vendored / 下载缓存都按这个版本对齐
export function bundledVersion() {
	try {
		return JSON.parse(readFileSync(join(pkgRoot, 'package.json'), 'utf8')).version ?? null
	} catch {
		return null
	}
}

/// 本地开发/源码安装: 是否带 Rust 源码 (决定能不能 cargo build)
export function hasRustSources() {
	return existsSync(join(pkgRoot, 'Cargo.toml'))
}

/// 包内 vendored 二进制: <pkgRoot>/vendor/<key>/bin/screeps-sim[.exe]
/// (安装时或首次运行时从 GitHub Release 落位; files 白名单包含 vendor/)
export function vendoredBinary(plat = detectPlatform()) {
	if (!plat.supported) return null
	const p = join(pkgRoot, 'vendor', plat.key, 'bin', exeName())
	return isRunnable(p) ? p : null
}

/// 全局缓存: ~/.cache/screeps-sim/<ver>/<key>/bin/<exe> (跨项目共享, 免重复下载)
function cacheBinary(plat = detectPlatform(), version = bundledVersion()) {
	if (!plat.supported || !version) return null
	return join(globalCacheRoot(), 'screeps-sim', `v${version}`, plat.key, 'bin', exeName())
}

function globalCacheRoot() {
	// 优先 XDG_CACHE_HOME, Windows 用 LOCALAPPDATA, 兜底 os.homedir()/.cache
	const home = spawnSync('node', ['-p', 'require("node:os").homedir()'], { encoding: 'utf8' })
	const homedir = home.status === 0 ? home.stdout.trim() : tmpdir()
	return process.env.XDG_CACHE_HOME || (process.platform === 'win32' && process.env.LOCALAPPDATA) || join(homedir, '.cache')
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

// ── GitHub Release 下载 (安装时 vendored 缺失 / 缓存未命中) ──────────

function fetchSync(url, { timeoutMs = 120_000, headers = {} } = {}) {
	// Node 22 原生 fetch 是异步的; CLI 这里用 curl 兜底同步实现 (三大平台都有;
	// Windows 10+ 自带 curl.exe)。返回 { ok, status, body?, toFile? }
	const args = ['-fsSL', '--max-time', String(Math.ceil(timeoutMs / 1000))]
	for (const [k, v] of Object.entries(headers)) args.push('-H', `${k}: ${v}`)
	const r = spawnSync('curl', [...args, url], { encoding: 'utf8', maxBuffer: 512 * 1024 * 1024 })
	return { ok: r.status === 0, status: r.status, body: r.stdout ?? '' }
}

function downloadTo(url, dest, { timeoutMs = 1_800_000 } = {}) {
	// 断点续传 + 重试: GitHub CDN 在弱网下常中途断流, 甚至整体拒绝连接 (时通时断)。
	// 循环 -C - 续传: 有进展就继续, 连续 stallLimit 轮零进展才放弃; 全程受 timeoutMs 总预算约束。
	const deadline = Date.now() + timeoutMs
	let lastSize = -1
	let stalled = 0
	const stallLimit = 4
	while (Date.now() < deadline) {
		const left = Math.max(30, Math.min(120, Math.ceil((deadline - Date.now()) / 1000)))
		spawnSync('curl', ['-fL', '-C', '-', '--max-time', String(left), '--retry', '2', '--retry-delay', '2', '-o', dest, url], { stdio: 'pipe' })
		const size = existsSync(dest) ? statSync(dest).size : 0
		if (size > 0 && size === lastSize) {
			// 文件大小不变: 可能恰好下完 (curl -C - 对完整文件直接 0 字节校验成功)
			if (spawnsCurlOk(url, size)) return true
		}
		if (size > lastSize) {
			stalled = 0
			lastSize = size
		} else if (++stalled >= stallLimit) {
			return false
		}
	}
	return existsSync(dest) && statSync(dest).size > 0
}

/// 下载是否完整: 远端 Content-Length 与本地文件比对 (HEAD 请求; 拿不到时视为完整)
function spawnsCurlOk(url, localSize) {
	const r = spawnSync('curl', ['-fsIL', '--max-time', '30', url], { encoding: 'utf8' })
	if (r.status !== 0) return true // 探测失败不下结论, 交给调用方下一轮
	for (const m of (r.stdout ?? '').matchAll(/content-length:\s*(\d+)/gi)) {
		const total = Number(m[1])
		if (total > 0) return localSize >= total
	}
	return true
}

/// 查 Release 最新版本号 (GitHub API redirects /releases/latest → /tag/<ver>)
function latestReleaseVersion() {
	const r = fetchSync(`https://github.com/${GITHUB_REPO}/releases/latest`, {
		timeoutMs: 30_000,
		headers: { 'User-Agent': 'screeps-sim-installer' },
	})
	// -f 下 302 不落 body; 用 -I 语义拿不到 → 换 API
	if (r.ok && /tag\/(v[\w.-]+)/.test(r.body)) return /tag\/(v[\w.-]+)/.exec(r.body)[1]
	const api = fetchSync(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`, {
		timeoutMs: 30_000,
		headers: { 'User-Agent': 'screeps-sim-installer' },
	})
	try {
		return JSON.parse(api.body).tag_name ?? null
	} catch {
		return null
	}
}

/// SHA256SUMS 资产内容: "hex  filename" 按行
function fetchChecksums(version) {
	const r = fetchSync(`https://github.com/${GITHUB_REPO}/releases/download/${version}/SHA256SUMS`, { timeoutMs: 60_000 })
	return r.ok ? r.body : null
}

/// 从 GitHub Release 下载当前平台的引擎二进制到 dest, 校验 SHA256 后返回 true
export function downloadReleaseBinary(dest, { version, key, quiet = false } = {}) {
	const plat = detectPlatform()
	const k = key ?? plat.key
	if (!targetByKey(k)) return { ok: false, reason: `未知平台 ${k}` }
	const ver = version ?? latestReleaseVersion()
	if (!ver) return { ok: false, reason: '无法确定最新 Release 版本 (网络不可达?)' }
	const asset = `screeps-sim-${k}-${ver.replace(/^v/, '')}`
	const base = `https://github.com/${GITHUB_REPO}/releases/download/${ver}`
	if (!quiet) console.error(`▸ 从 GitHub Release 下载引擎 ${ver} (${k})…`)
	const dl = join(dirname(dest), `.dl-${asset}`)
	mkdirSync(dirname(dest), { recursive: true })
	if (!downloadTo(`${base}/${asset}`, dl)) {
		rmSync(dl, { force: true })
		return { ok: false, reason: `下载失败: ${base}/${asset}` }
	}
	// SHA256 校验 (资产清单由 CI 生成; 拿不到清单时放行但提示)
	const sums = fetchChecksums(ver)
	if (sums) {
		const hex = createHash('sha256').update(readFileSync(dl)).digest('hex')
		const line = sums.split('\n').find((l) => l.trimEnd().endsWith(asset))
		const want = line?.trim().split(/\s+/)[0]?.toLowerCase()
		if (want && want !== hex) {
			rmSync(dl, { force: true })
			return { ok: false, reason: `SHA256 不符 (期望 ${want}, 实际 ${hex}) — 下载不完整或被篡改` }
		}
	} else if (!quiet) {
		console.error('  ! 无法获取 SHA256SUMS, 跳过完整性校验')
	}
	mkdirSync(dirname(dest), { recursive: true })
	renameSync(dl, dest)
	chmodSync(dest, 0o755)
	return { ok: true, version: ver }
}

/// 确保引擎二进制就位: 已 vendored → 用; 缓存命中 → 用; 都没有 → 下载到缓存。
/// 返回 { path, source } 或 { error }
export function acquireBinary({ download = true, quiet = false } = {}) {
	const plat = detectPlatform()
	const cached = cacheBinary(plat)
	if (cached && isRunnable(cached)) return { path: cached, source: `全局缓存 v${bundledVersion()}` }
	const vendor = vendoredBinary(plat)
	if (vendor) return { path: vendor, source: `包内 vendor/${plat.key}` }
	if (!download) return { error: '未 vendored 且未允许下载' }
	if (!plat.supported) return { error: `当前平台 ${plat.platform}/${plat.arch} 没有预编译引擎` }
	const target = cached ?? join(pkgRoot, 'vendor', plat.key, 'bin', exeName())
	const r = downloadReleaseBinary(target, { quiet })
	if (!r.ok) return { error: r.reason }
	return { path: target, source: r.version ? `GitHub Release ${r.version}` : 'GitHub Release' }
}

/// 全部候选 (按优先级): 返回 { path, source } 便于 doctor 展示来源
export function candidateBinaries() {
	const out = []
	if (process.env.SCREEPS_SIM_BIN) out.push({ path: process.env.SCREEPS_SIM_BIN, source: '环境变量 SCREEPS_SIM_BIN' })
	const local = join(pkgRoot, 'target', 'release', exeName())
	// 源码树里已有构建产物 → 视为开发者 checkout, 本地构建优先 (改代码即时生效)
	if (isRunnable(local)) out.push({ path: local, source: '源码构建 target/release' })
	const cached = cacheBinary()
	if (cached && isRunnable(cached)) out.push({ path: cached, source: `全局缓存 v${bundledVersion()}` })
	const vendor = vendoredBinary()
	if (vendor) out.push({ path: vendor, source: `包内 vendor/` })
	return out
}

/// 兼容旧调用: 只取候选路径
export function candidateBinaryPaths() {
	return candidateBinaries().map((c) => c.path)
}

/// 找到可用的引擎二进制 (不构建不下载); 没找到返回 null
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
			platformPackageInstalled: !!vendoredBinary(plat),
			sourceTree: hasRustSources(),
			invalidEnvPath: explicit,
		}
	}
	return {
		ok: !!found,
		path: found?.path ?? null,
		source: found?.source ?? null,
		platform: plat,
		/** 二进制是否已就位 (vendored / 缓存 / 源码构建) */
		platformPackageInstalled: !!vendoredBinary(plat) || (cached() && isRunnable(cached())),
		/** 是否是源码安装 (带 Cargo.toml, 可本地编译) */
		sourceTree: hasRustSources(),
	}
}

// 兼容旧名: 原"平台包是否已安装"语义 → 引擎二进制是否已就位 (vendored/缓存)
function cached() {
	return cacheBinary()
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
				`或设置 SCREEPS_SIM_BIN 指向已构建的二进制。`
		)
	}
	if (r.status !== 0) {
		throw new Error(`cargo build 失败 (退出码 ${r.status})`)
	}
}

/// 确保二进制存在: 已有 → 用; 源码安装 → cargo build; 否则 → 从 GitHub Release 下载
export function ensureBinary({ build = true, download = true } = {}) {
	checkExplicitBin()
	const found = findBinary()
	if (found) return found.path
	// 源码安装优先本地构建 (版本与源码一致); 纯安装形态走下载
	if (build && hasRustSources() && (process.env.SCREEPS_SIM_NO_DOWNLOAD ?? '') === '') {
		try {
			buildBinary()
			const again = findBinary()
			if (again) return again.path
		} catch (e) {
			if (!download) throw e
			console.error(`! 本地构建失败, 回退到 GitHub Release 下载 (${e.message})`)
		}
	}
	if (download && plat_ok()) {
		const r = acquireBinary({ quiet: false })
		if (r.path) return r.path
	}
	throw installError(missingBinaryMessage())
}

function plat_ok() {
	return detectPlatform().supported
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
		lines.push(`当前平台: ${plat.platform}/${plat.arch}${plat.libc ? ` (${plat.libc})` : ''}`)
		lines.push(`解决: ① 重跑一次命令 (自动从 GitHub Release 下载引擎, 带SHA256校验)  ② 设置 SCREEPS_SIM_BIN=<二进制路径>`)
		if (!networkHint()) lines.push(`    (或检查网络: 下载来自 https://github.com/${GITHUB_REPO}/releases)`)
	} else {
		lines.push(`当前平台 ${plat.platform}/${plat.arch} 没有官方预编译包: 请自行 cargo build --release, 或设置 SCREEPS_SIM_BIN 指向已有二进制。`)
	}
	lines.push(`诊断详情: screeps-sim doctor`)
	return lines.join('\n')
}

function networkHint() {
	return fetchSync(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`, { timeoutMs: 8_000, headers: { 'User-Agent': 'screeps-sim-installer' } }).ok
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
		let
			stderr = ''
		child.stdout.on('data', (d) => (stdout += d))
		child.stderr.on('data', (d) => (stderr += d))
		child.on('close', (status) => resolve({ status, stdout, stderr }))
		child.stdin.write(stdinText ?? '')
		child.stdin.end()
	})
}
