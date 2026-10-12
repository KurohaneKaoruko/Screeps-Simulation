#!/usr/bin/env node
// screeps-sim 入口:
//   screeps-sim test [patterns...] [options]   .sim.ts 用例测试 runner
//   screeps-sim build                           构建底层 Rust 引擎 (cargo build --release)
//   screeps-sim <其他参数>                       透传给 Rust 二进制 (场景直跑, 见 --help)
import { runTests, runChecks, parseTestArgs } from '../runner/main.mjs'
import { buildBinary, ensureBinary, forwardToBinary } from '../runner/binary.mjs'

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
	console.error(String(err?.stack || err))
	process.exit(1)
}

function printHelp() {
	console.log(`screeps-sim — Screeps 本地高速模拟器

用法:
  screeps-sim test [patterns...] [options]    运行 .sim.ts 模拟用例 (类单元测试)
  screeps-sim check [patterns...] [--strict]  静态检查场景定义 (不跑模拟, 编译器式诊断)
  screeps-sim build                           构建底层 Rust 引擎
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
  SCREEPS_SIM_BIN   显式指定 Rust 二进制路径 (跳过自动构建)`)
}
