// 重新生成 runner/screeps-constants.mjs (从 src/js/screeps.js 的常量块提取)。
// selftest 会校验生成文件与 shim 同步; shim 常量变化后跑: node runner/gen-constants.mjs
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

export function extractShimConstants(src) {
	const start = src.indexOf('const C = {')
	if (start < 0) throw new Error('screeps.js 中未找到常量块 const C = {')
	let depth = 0
	let end = -1
	for (let i = start + 'const C = {'.length - 1; i < src.length; i++) {
		if (src[i] === '{') depth++
		else if (src[i] === '}') {
			depth--
			if (depth === 0) {
				end = i
				break
			}
		}
	}
	return [...src.slice(start, end + 1).matchAll(/^\t([A-Z][A-Z0-9_]+):/gm)].map((m) => m[1])
}

// 作为脚本直接运行时才写入 (被 import 时只提供 extractShimConstants, 无副作用)
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
	const src = readFileSync(new URL('../src/js/screeps.js', import.meta.url), 'utf8')
	const names = extractShimConstants(src)
	const out =
		'// 由 src/js/screeps.js 的常量块自动生成 (selftest 校验同步, 勿手改)\n' +
		'// 重新生成: node runner/gen-constants.mjs\n' +
		`export const SCREEPS_CONSTANTS = new Set(${JSON.stringify(names)})\n`
	writeFileSync(new URL('./screeps-constants.mjs', import.meta.url), out)
	console.log(`生成 ${names.length} 个常量 → runner/screeps-constants.mjs`)
}
