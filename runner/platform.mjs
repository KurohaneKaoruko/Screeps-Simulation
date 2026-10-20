// 平台探测: 把 (os, cpu, libc) 映射到平台 key, 供二进制定位 / Release 资产构建共用。
//
// 平台 key 约定: `<os>-<cpu>[-<libc>]`, 对应 GitHub Release 资产 `screeps-sim-<key>-<版本>`。
// runner 首次运行时按当前平台 key 自动下载对应资产 (SHA256 校验) —— 安装主包时零 Rust 工具链。
import { existsSync } from 'node:fs'

/// 全部发布目标 (CI 矩阵 / 平台包 optionalDependencies 的唯一真相源)
export const TARGETS = [
	{ key: 'linux-x64-gnu', os: 'linux', cpu: 'x64', libc: 'glibc', rustTarget: 'x86_64-unknown-linux-gnu' },
	{ key: 'linux-x64-musl', os: 'linux', cpu: 'x64', libc: 'musl', rustTarget: 'x86_64-unknown-linux-musl' },
	{ key: 'linux-arm64-gnu', os: 'linux', cpu: 'arm64', libc: 'glibc', rustTarget: 'aarch64-unknown-linux-gnu' },
	{ key: 'linux-arm64-musl', os: 'linux', cpu: 'arm64', libc: 'musl', rustTarget: 'aarch64-unknown-linux-musl' },
	{ key: 'darwin-x64', os: 'darwin', cpu: 'x64', libc: null, rustTarget: 'x86_64-apple-darwin' },
	{ key: 'darwin-arm64', os: 'darwin', cpu: 'arm64', libc: null, rustTarget: 'aarch64-apple-darwin' },
	{ key: 'win32-x64', os: 'win32', cpu: 'x64', libc: null, rustTarget: 'x86_64-pc-windows-msvc' },
	{ key: 'win32-arm64', os: 'win32', cpu: 'arm64', libc: null, rustTarget: 'aarch64-pc-windows-msvc' },
]

export const TARGET_KEYS = TARGETS.map((t) => t.key)

export function targetByKey(key) {
	return TARGETS.find((t) => t.key === key) ?? null
}

export function packageNameFor(key) {
	return 'screeps-sim-' + key
}

/// linux 的 libc 判定: Node 自报 glibc 版本即 glibc; 否则查 musl 加载器 (Alpine 等)
export function detectLibc(platform = process.platform) {
	if (platform !== 'linux') return null
	let header = {}
	try {
		header = process.report?.getReport()?.header ?? {}
	} catch {
		/* --no-report 等场景下拿不到, 走文件探测 */
	}
	if (header.glibcVersionRuntime || header.glossVersion) return 'glibc'
	for (const p of [
		'/lib/ld-musl-x86_64.so.1',
		'/lib/ld-musl-aarch64.so.1',
		'/lib/libc.musl-x86_64.so.1',
		'/usr/lib/libc.musl-x86_64.so.1',
	]) {
		if (existsSync(p)) return 'musl'
	}
	return 'glibc'
}

/// 当前运行平台 → 平台包描述 (不受支持时 supported:false, 由调用方给出可执行的排障提示)
export function detectPlatform({
	platform = process.platform,
	arch = process.arch,
	libc = detectLibc(platform),
} = {}) {
	const match = TARGETS.find((t) => t.os === platform && t.cpu === arch && (platform !== 'linux' || t.libc === libc))
	const base = { platform, arch, libc }
	if (!match) return { ...base, key: null, packageName: null, rustTarget: null, supported: false }
	return { ...base, ...match, packageName: packageNameFor(match.key), supported: true }
}
