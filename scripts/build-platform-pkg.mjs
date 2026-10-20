// 构建当前 (或指定) 平台的引擎二进制, 产出 GitHub Release 资产 (裸二进制, CI 上传用)。
//
//   node scripts/build-platform-pkg.mjs                      # 当前平台, 需要 cargo build
//   node scripts/build-platform-pkg.mjs --skip-build         # 复用已有 target/ 产物 (本机验证用)
//   node scripts/build-platform-pkg.mjs --key darwin-x64 --rust-target x86_64-apple-darwin
//   node scripts/build-platform-pkg.mjs --cross              # 用 cross (docker) 交叉编译 linux 目标
//   node scripts/build-platform-pkg.mjs --out dist/platform
//
// 产物: <out>/screeps-sim-<key>-<version> (裸二进制, 文件名即 Release 资产名)
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, statSync, readFileSync, chmodSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { TARGETS, TARGET_KEYS, detectPlatform } from '../runner/platform.mjs'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')

// ── 参数 ────────────────────────────────────────────────
const argv = process.argv.slice(2)
const flag = (name) => argv.includes(`--${name}`)
const opt = (name, fallback) => {
	const i = argv.indexOf(`--${name}`)
	return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback
}

const plat = detectPlatform()
const key = opt('key', plat.key)
if (!key || !TARGET_KEYS.includes(key)) {
	console.error(`未知平台 key: ${key ?? '(当前平台不在已发布列表)'} · 可选: ${TARGET_KEYS.join(', ')}`)
	process.exit(1)
}
const target = TARGETS.find((t) => t.key === key)
const rustTarget = opt('rust-target', target.rustTarget)
const outDir = join(root, opt('out', 'dist/platform'))
const skipBuild = flag('skip-build')
const strip = !flag('no-strip')
const exeName = process.platform === 'win32' ? 'screeps-sim.exe' : 'screeps-sim'
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))

// ── 1. 引擎二进制 ───────────────────────────────────────
function hostTarget() {
	const r = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
	return /^host:\s*(\S+)/m.exec(r.stdout ?? '')?.[1] ?? null
}

const native = !rustTarget || rustTarget === hostTarget()
const binPath = native
	? join(root, 'target', 'release', exeName)
	: join(root, 'target', rustTarget, 'release', exeName)

if (!skipBuild) {
	// cross: docker 里的交叉工具链 (CI 里编 linux 交叉目标用, 免装 gcc-aarch64 等)
	const tool = flag('cross') ? 'cross' : 'cargo'
	const args = ['build', '--release']
	if (!native) args.push('--target', rustTarget)
	console.log(`▸ ${tool} ${args.join(' ')}  (${target.key})`)
	const r = spawnSync(tool, args, { cwd: root, stdio: 'inherit' })
	if (r.error) {
		console.error(`无法调用 ${tool} (${r.error.message}) · CI 里需先 rustup target add ${rustTarget}`)
		process.exit(1)
	}
	if (r.status !== 0) {
		console.error(`${tool} build 失败 (退出码 ${r.status})`)
		process.exit(r.status ?? 1)
	}
}
if (!existsSync(binPath)) {
	console.error(`找不到构建产物: ${binPath}${skipBuild ? ' (去掉 --skip-build 重新构建)' : ''}`)
	process.exit(1)
}

// ── 2. strip (66MB → ~30MB, 分发体积减半) ───────────────
mkdirSync(outDir, { recursive: true })
// 资产名与 runner/binary.mjs 的 downloadReleaseBinary 对齐: screeps-sim-<key>-<version>
const assetName = `screeps-sim-${key}-${pkg.version}${process.platform === 'win32' ? '.exe' : ''}`
const dest = join(outDir, assetName)
copyFileSync(binPath, dest)
if (strip && process.platform !== 'win32') {
	const r = spawnSync('strip', [dest], { stdio: 'inherit' })
	if (r.status !== 0) console.warn(`! strip 失败 (${r.error?.message ?? r.status}), 保留未剥离二进制 (包更大)`)
}
chmodSync(dest, 0o755)
const sizeBytes = statSync(dest).size

// ── 3. 清单 (CI 汇总 SHA256SUMS / 校验用) ───────────────
const manifest = {
	key,
	version: pkg.version,
	rustTarget,
	asset: assetName,
	path: `dist/platform/${assetName}`,
	sizeBytes,
	sha256: createHash('sha256').update(readFileSync(dest)).digest('hex'),
}
writeFileSync(join(outDir, `${key}.json`), JSON.stringify(manifest, null, '\t') + '\n')

console.log(
	`✓ screeps-sim-${key}@${pkg.version}  ${(sizeBytes / 1024 / 1024).toFixed(1)} MB (strip=${strip})` +
		`\n  资产    ${manifest.path}` +
		`\n  清单    dist/platform/${key}.json`
)
