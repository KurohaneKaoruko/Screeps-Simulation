// 版本/依赖一致性校验 (prepack + npm test 用):
//   1. package.json 版本 == Cargo.toml 版本 (引擎与 npm 包同版本发布)
//   2. optionalDependencies 覆盖 runner/platform.mjs 的全部发布目标, 版本严格等于主包版本
//   3. 平台包名与 SCOPE 一致
//
// 用法: node scripts/version-check.mjs [--write]   (--write: 把 Cargo.toml 版本对齐到 package.json)
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { SCOPE, TARGET_KEYS, packageNameFor } from '../runner/platform.mjs'

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const cargoToml = readFileSync(join(root, 'Cargo.toml'), 'utf8')
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(cargoToml)?.[1] ?? null
const write = process.argv.includes('--write')

const problems = []

if (!cargoVersion) problems.push('Cargo.toml 里找不到 version 字段')
if (cargoVersion && cargoVersion !== pkg.version) {
	if (write) {
		writeFileSync(join(root, 'Cargo.toml'), cargoToml.replace(/^version\s*=\s*"[^"]+"/m, `version = "${pkg.version}"`))
		console.log(`· Cargo.toml 版本 ${cargoVersion} → ${pkg.version}`)
	} else {
		problems.push(`版本不一致: Cargo.toml ${cargoVersion} ≠ package.json ${pkg.version} (跑 node scripts/version-check.mjs --write 对齐)`)
	}
}

const optional = pkg.optionalDependencies ?? {}
const expected = new Map(TARGET_KEYS.map((k) => [packageNameFor(k), pkg.version]))
for (const [name, want] of expected) {
	const got = optional[name]
	if (got === undefined) problems.push(`optionalDependencies 缺少 ${name}`)
	else if (got !== want) problems.push(`optionalDependencies ${name} 版本应为 ${want} (精确锁定, 防平台错配), 当前 ${got}`)
}
for (const name of Object.keys(optional)) {
	if (!expected.has(name)) problems.push(`optionalDependencies 多余条目 ${name} (已知目标: ${[...expected.keys()].join(', ')})`)
}

if (problems.length) {
	console.error(`版本校验失败 (${problems.length}):`)
	for (const p of problems) console.error(`  · ${p}`)
	console.error(`\n平台包 scope: ${SCOPE} (发布前需改成自己拥有的 scope, 见 runner/platform.mjs SCOPE)`)
	process.exit(1)
}
console.log(`版本校验通过: ${pkg.name}@${pkg.version} (Cargo.toml 同版本) · 平台包 ${Object.keys(optional).length} 个 ${SCOPE}/<platform>`)
