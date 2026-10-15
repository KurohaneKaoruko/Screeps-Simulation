// 构建当前 (或指定) 平台的预编译二进制, 并打包成 npm 平台包 `@screeps-sim/<key>`。
//
//   node scripts/build-platform-pkg.mjs                      # 当前平台, 需要 cargo build
//   node scripts/build-platform-pkg.mjs --skip-build         # 复用已有 target/ 产物 (本机验证用)
//   node scripts/build-platform-pkg.mjs --key linux-arm64-gnu --rust-target aarch64-unknown-linux-gnu
//   node scripts/build-platform-pkg.mjs --cross              # 用 cross (docker) 交叉编译 linux 目标
//   node scripts/build-platform-pkg.mjs --out dist/platform
//
// 产物: <out>/<包名 tarball>.tgz + <out>/<key>.json 清单 (CI 据此上传 Release / 发布 npm)。
// 交叉编译需要先 `rustup target add <rust-target>`; musl 目标还需要 V8 的 musl 预编译库
// (deno_core 的 rusty_v8 是否提供见构建报错, 缺失时该目标跳过发布)。
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, rmSync, statSync, writeFileSync, chmodSync, readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { TARGETS, TARGET_KEYS, detectPlatform, packageNameFor } from '../runner/platform.mjs'

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

// ── 2. strip (66MB → ~30MB, 平台包体积减半) ───────────────
const stripped = join(outDir, `screeps-sim-${key}${process.platform === 'win32' ? '.exe' : ''}`)
mkdirSync(outDir, { recursive: true })
copyFileSync(binPath, stripped)
if (strip && process.platform !== 'win32') {
	const r = spawnSync('strip', [stripped], { stdio: 'inherit' })
	if (r.status !== 0) console.warn(`! strip 失败 (${r.error?.message ?? r.status}), 保留未剥离二进制 (包更大)`)
}
chmodSync(stripped, 0o755)
const sizeBytes = statSync(stripped).size

// ── 3. 组装平台包 ───────────────────────────────────────
const stageDir = join(outDir, key)
rmSync(stageDir, { recursive: true, force: true })
mkdirSync(join(stageDir, 'bin'), { recursive: true })
copyFileSync(stripped, join(stageDir, 'bin', exeName))
chmodSync(join(stageDir, 'bin', exeName), 0o755)

const platformPkg = {
	name: packageNameFor(key),
	version: pkg.version,
	description: `${pkg.name} 预编译引擎二进制 (${key}) · 不含 JS 源码, 需配合主包使用`,
	license: pkg.license,
	repository: pkg.repository,
	homepage: pkg.homepage,
	// npm 按 os/cpu/libc 自动只装当前平台的包: 其余 optionalDependencies 会被静默跳过
	os: [target.os],
	cpu: [target.cpu],
	...(target.libc ? { libc: [target.libc] } : {}),
	files: ['bin'],
	exports: {
		'./package.json': './package.json',
		[`./bin/${exeName}`]: `./bin/${exeName}`,
	},
	publishConfig: { access: 'public' },
}
writeFileSync(join(stageDir, 'package.json'), JSON.stringify(platformPkg, null, '\t') + '\n')
writeFileSync(
	join(stageDir, 'README.md'),
	`# ${platformPkg.name}\n\n${platformPkg.description}\n\n由 [${pkg.name}](${pkg.homepage ?? ''}) 的发布流水线自动生成, 请勿手动修改或单独安装。\n`
)

// ── 4. 打包 + 清单 ──────────────────────────────────────
const npmCmd = process.platform === 'win32' ? 'npm.cmd' : 'npm'
console.log(`  ▸ ${npmCmd} pack --pack-destination ${outDir} (in ${stageDir})`)
const packed = spawnSync(npmCmd, ['pack', '--pack-destination', outDir], { cwd: stageDir, encoding: 'utf8', shell: true })
if (packed.status !== 0) {
	console.error('npm pack 失败 (status=' + packed.status + '):')
	console.error('  stdout:', packed.stdout)
	console.error('  stderr:', packed.stderr)
	process.exit(1)
}
const tgzName = (packed.stdout || '').trim().split('\n').pop().trim()
const tgz = join(outDir, tgzName)
const manifest = {
	key,
	packageName: platformPkg.name,
	version: pkg.version,
	rustTarget,
	tgz: `dist/platform/${tgzName}`,
	sizeBytes,
	sha256: createHash('sha256').update(readFileSync(tgz)).digest('hex'),
}
writeFileSync(join(outDir, `${key}.json`), JSON.stringify(manifest, null, '\t') + '\n')

console.log(
	`✓ ${platformPkg.name}@${pkg.version}  ${(sizeBytes / 1024 / 1024).toFixed(1)} MB (strip=${strip})` +
		`\n  tgz     ${manifest.tgz}` +
		`\n  清单    dist/platform/${key}.json`
)
