// 版本一致性校验 (npm test 用):
//   package.json 版本 == Cargo.toml 版本 (引擎与 JS runner 同版本发布)
//
// 用法: node scripts/version-check.mjs [--write]   (--write: 把 Cargo.toml 版本对齐到 package.json)
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

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

if (problems.length) {
	console.error(`版本校验失败 (${problems.length}):`)
	for (const p of problems) console.error(`  · ${p}`)
	process.exit(1)
}
console.log(`版本校验通过: ${pkg.name}@${pkg.version} (Cargo.toml 同版本)`)
