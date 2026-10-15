#!/usr/bin/env node
// screeps-sim 入口:
//   screeps-sim test [patterns...] [options]   .sim.ts 用例测试 runner
//   screeps-sim check [patterns...] [options]  场景/bundle 静态检查
//   screeps-sim build                           构建底层 Rust 引擎 (cargo build --release)
//   screeps-sim doctor                          诊断: 平台/二进制/项目配置 (装不上时先跑这个)
//   screeps-sim version                         版本信息 (不启动引擎)
//   screeps-sim <其他参数>                       透传给 Rust 二进制 (场景直跑, 见 --help)
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { runTests, runChecks, parseTestArgs } from '../runner/main.mjs'
import { buildBinary, ensureBinary, forwardToBinary } from '../runner/binary.mjs'
import { printDoctor } from '../runner/doctor.mjs'

const args = process.argv.slice(2)

try {
	switch (args[0]) {
		case 'test': {
			const opts = parseTestArgs(args.slice(1))
			await runTests(opts)
			break
		}
		case 'check': {
			// 静态检查: 不跑模拟, 不需要引擎二进制
			const opts = parseTestArgs(args.slice(1))
			await runChecks(opts)
			break
		}
		case 'build': {
			buildBinary()
			break
		}
		case 'doctor': {
			process.exit(printDoctor())
			break
		}
		case 'version': {
			printVersion()
			break
		}
		case 'help':
		case '--help':
		case '-h': {
			printHelp()
			break
		}
		default: {
			// 无子命令: 透传 (保持 `screeps-sim --scenario xx.json` 直跑场景)
			forwardToBinary(args)
		}
	}
} catch (err) {
	// 安装/环境类错误 (err.code) 只打消息: 提示本身可执行, 调用栈只会淹没它
	console.error(err?.code === 'ERR_SIM_NO_BINARY' ? err.message : String(err?.stack || err))
	process.exit(1)
}

/// 版本: npm 包版本 + 平台包 + 已解析到的引擎版本 (拿不到引擎也不报错)
function printVersion() {
	const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))
	const lines = [`${pkg.name} ${pkg.version} (node ${process.version}, ${process.platform}/${process.arch})`]
	try {
		const bin = ensureBinary()
		const r = spawnSync(bin, ['--version'], { encoding: 'utf8', timeout: 10_000 })
		lines.push(`engine ${String(r.stdout || '').trim() || '(未知)'}  ← ${bin}`)
	} catch (err) {
		lines.push(`engine 不可用: ${String(err.message).split('\n')[0]}`)
	}
	console.log(lines.join('\n'))
}

function printHelp() {
	console.log(`screeps-sim — Screeps 本地高速模拟器

用法:
  screeps-sim test [patterns...] [options]    运行 .sim.ts 模拟用例 (类单元测试)
  screeps-sim check [patterns...] [--strict]  静态检查场景定义 (不跑模拟, 编译器式诊断)
  screeps-sim build                           构建底层 Rust 引擎 (源码安装时用)
  screeps-sim doctor                          诊断平台/引擎二进制/项目配置
  screeps-sim version                         版本信息 (npm 包 + 引擎)
  screeps-sim --scenario <json|-> [选项]       直跑单个场景 JSON (透传给引擎)

check 检查项 (模拟启动前):
  房名/坐标/部件/结构类型/断言类型/JSON memory 等硬错误
  可疑写法警告: RCL 超限结构(将被 off)、同格多结构、墙上的对象、房间不连通、
  不可能达成的断言、seed=0 不可复现、alwaysVisible 调试残留等

test 选项:
  [patterns...]     文件 / 目录 / 通配符, 缺省扫描当前目录 (**/*.sim.ts, 跳过 node_modules/target 等)
  --bundle, -b <p>  bot bundle 路径 (默认: 用例内 bundle 字段, 再退回 ./dist/main.js)
  --ticks, -t <n>   覆盖所有用例的 tick 数 (快速冒烟)
  --jobs, -j <n>    并行运行 n 个引擎进程 (含 cpu*Below 断言时建议 -j 1)
  --list, -l        只列出发现的用例, 不运行
  --verbose, -v     显示 bot console 输出与引擎 stderr
  --no-bundle-check bundle 预检错误只提示, 不判失败 (也可在 package.json "screeps-sim".bundleCheck: false)

环境变量:
  SCREEPS_SIM_BIN   显式指定 Rust 二进制路径 (跳过平台包与自动构建)`)
}
