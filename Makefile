# 云笺 · 构建与门禁入口
#
# 本文件是仓库里唯一定义门禁的地方。CI 工作流与 git hooks 都只调用这里的目标名，
# 因此「本机跑绿」与「CI 跑绿」永远是同一组命令，不会各自漂移。
#
# 所有命令与参数一律用 `:=` 立即展开赋值，**绝不使用 `?=`**。`?=` 允许环境变量
# 覆盖，于是一句 `CLIPPY_FLAGS= make lint` 就能把门禁悄悄削成空操作。门禁不接受
# 来自环境的削弱。

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO := cargo
OXFMT := oxfmt
PRE_COMMIT := pre-commit

# `-D warnings` 是门禁本身而不是风格偏好：工作区的 stdout 禁令
# （`print_stdout = "deny"` 等）全靠它把 deny 级 lint 变成非零退出码。
# `--all-targets` 让测试与 example 也一并受检。
CLIPPY_FLAGS := --workspace --all-targets -- -D warnings

# oxfmt 默认只读 `.gitignore` 与 `.prettierignore`，**不会自动发现 `.oxfmtignore`**，
# 所以必须显式传入。同时保留 `.gitignore`，否则 `target/` 与 `.omo/` 会被一并格式化。
OXFMT_ARGS := --ignore-path .oxfmtignore --ignore-path .gitignore .

# `check` 与 `ci` 共用同一份门禁定义。用一个变量展开两处，从机制上杜绝
# 「本地 check 通过、CI 却多跑一步」这种漂移。
# `frontend-test` 必须在门禁里，不是可选补充：自绘标题栏的时序缺陷（StrictMode 双调用下
# 卸载后写状态、订阅泄漏）与非 Tauri 降级只有 Vitest 验得到，Rust 侧一条都覆盖不到。
# 不进 GATE 就等于那些断言只在有人手动想起来的时候才跑。
GATE := fmt-check lint test frontend-test

# 语料门禁跑的样本规模。10k 是方案为 CI 指定的规模：足够大到让索引行为与真实语料同形
# （两字查询在 19 首上无论怎么查都是零点几毫秒，看不出路径退化），又足够小到能在几秒内跑完。
CORPUS_GATE_SCALE := 10000

.DEFAULT_GOAL := help
# 前端构建产物的存在标记。**这是一个真实的编译期前置条件，不是便利设施**：
# `crates/yunjian-app` 的 `generate_context!` 在 `build.frontendDist` 指向的目录缺失时
# 直接 panic（tauri-codegen 的 "this path doesn't exist"），而 `cargo test --workspace`
# 与 `cargo clippy --workspace` 都要编译那个 crate。于是一个没跑过前端构建的新检出
# 会让全部测试**无法编译**，失败信息还落在一段 codegen panic 里。
#
# 不能改用「往版本库里放一个 dist/.gitkeep」绕过：`vite build` 会先清空 outDir，
# 占位文件在第一次构建后就没了（已实测）。所以只能真的构建。
FRONTEND_DIST := app/dist/index.html

NPM := npm

.PHONY: help fmt fmt-rust fmt-oxfmt fmt-check lint test build check ci corpus-gate \
	corpus-artifact hooks frontend frontend-test

help: ## 列出全部可用目标
	@echo "云笺 · make 目标"
	@echo
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN { FS = ":.*?## " } { printf "  %-10s %s\n", $$1, $$2 }'
	@echo
	@echo "门禁：make ci == $(GATE)"
	@echo "      pre-push 钩子与 CI 跑的都是这一条，不存在第二套标准。"

fmt: fmt-rust fmt-oxfmt ## 格式化 Rust 代码与配置/文档（cargo fmt + oxfmt）

fmt-rust:
	@echo "==> cargo fmt --all"
	@$(CARGO) fmt --all

fmt-oxfmt:
	@command -v $(OXFMT) >/dev/null 2>&1 || { \
		echo "缺少 oxfmt。安装：mise use -g oxfmt，或 cargo install oxfmt" >&2; \
		exit 1; \
	}
	@echo "==> oxfmt（Markdown / YAML / JSON / TOML）"
	@$(OXFMT) $(OXFMT_ARGS)

# 刻意不写成 `fmt-check: fmt-check-rust fmt-check-oxfmt`：make 在第一个失败的
# 前置目标上就会停下，于是只报 Rust 或只报 Markdown，改一处再跑一次才看到另一处。
# 这里把两步都跑完再汇总退出码，一次就能看全。
fmt-check: ## 校验格式；Rust 与 oxfmt 两步都会跑完，一次报出全部问题
	@command -v $(OXFMT) >/dev/null 2>&1 || { \
		echo "缺少 oxfmt。安装：mise use -g oxfmt，或 cargo install oxfmt" >&2; \
		exit 1; \
	}
	@status=0; \
	echo "==> cargo fmt --all --check"; \
	$(CARGO) fmt --all --check || status=1; \
	echo "==> oxfmt --check"; \
	$(OXFMT) --check $(OXFMT_ARGS) || status=1; \
	if [ "$$status" -ne 0 ]; then \
		echo "格式校验未通过：运行 make fmt 修复后重试。" >&2; \
	fi; \
	exit "$$status"

frontend: ## 构建桌面端前端（app/）。lint / test / build 会在产物缺失时自动先跑它
	@command -v $(NPM) >/dev/null 2>&1 || { \
		echo "缺少 npm。桌面端前端是 cargo 的编译期前置条件，不是可选项。" >&2; \
		echo "安装 Node.js（>= 20）后重试：mise use -g node" >&2; \
		exit 1; \
	}
	@echo "==> npm ci（app/）"
	@cd app && if [ -f package-lock.json ]; then $(NPM) ci; else $(NPM) install; fi
	@echo "==> npm run build（app/）"
	@cd app && $(NPM) run build

# 产物缺失时才构建。写成文件目标而不是 .PHONY，这样已经构建过的树上零开销。
$(FRONTEND_DIST):
	@$(MAKE) --no-print-directory frontend

# 前端测试。依赖 `$(FRONTEND_DIST)` 不只是为了拿到 node_modules：`contracts.test.ts` 里
# 「构建产物的样式表里没有那个 Electron 拖动属性」这条断言要读真实产物，而它刻意
# **不跳过**产物缺失的情况（跳过会让门禁里最需要它的那次执行变成空操作）。
frontend-test: | $(FRONTEND_DIST) ## 跑桌面端前端测试（Vitest）
	@echo "==> npm test（app/）"
	@cd app && $(NPM) test

# order-only 前置（`|` 右侧）：只要求产物存在，不因它比源码旧就重跑。
# 前端源码改了要重新构建的是开发者自己的事（`make frontend`），
# 把它做成时间戳依赖会让每次 `make test` 都可能触发一次 npm，门禁时长随之抖动。
lint: | $(FRONTEND_DIST)
test: | $(FRONTEND_DIST)
build: | $(FRONTEND_DIST)

lint: ## clippy 覆盖全工作区与全 target，出现警告即失败
	@echo "==> cargo clippy $(CLIPPY_FLAGS)"
	@$(CARGO) clippy $(CLIPPY_FLAGS)

test: ## 跑全工作区测试。不加过滤，doctest 才会被执行
	@echo "==> cargo test --workspace"
	@$(CARGO) test --workspace

# 语音特性（`--features voice`）刻意不进任何门禁：它会拉起需要 libclang 与
# 网络下载的原生依赖，且分发物许可与默认构建不同。要验它请显式指定特性。
build: ## 发布构建。刻意不属于 ci：耗时长，且不是正确性门禁
	@echo "==> cargo build --release"
	@$(CARGO) build --release

check: $(GATE) ## 本地全量校验，与 ci 完全等价
	@echo "全部通过：$(GATE)"

ci: $(GATE) ## 唯一门禁：pre-push 钩子与 CI 都只跑这一条
	@echo "门禁通过：$(GATE)"

# 语料门禁。**刻意不属于 `ci`**：`ci` 是 pre-push 钩子跑的那一条，加进去会让每次推送
# 都重建一个一万首的库；语料另有专用工作流（`.github/workflows/corpus.yml`）与每月
# 定时复验。工作流只调用本目标，因此「本机跑绿」与「语料 CI 跑绿」仍然是同一组命令。
#
# 顺序是有意义的：许可判定在最前（不合许可的源根本不该被读进来），契约在质量报告之前
# （检索坏了就不必再看缺陷计数），工件漂移紧跟在生成它的那一步之后。
corpus-gate: ## 语料门禁：许可、10k 规模黄金查询契约、质量基线、工件漂移、契约单副本
	@echo "==> 契约在整个仓库里只有一份"
	@count=$$(find . -name queries.toml -not -path "./target/*" -not -path "./.git/*" -not -path "./.omo/*" -not -path "./.worktrees/*" | wc -l); \
	if [ "$$count" -ne 1 ]; then \
		echo "仓库里有 $$count 份 queries.toml。契约只有一处，六方共同消费；" >&2; \
		echo "消费方请引用 crates/yunjian-core/tests/queries.toml，不要复制。" >&2; \
		find . -name queries.toml -not -path "./target/*" -not -path "./.git/*" -not -path "./.omo/*" -not -path "./.worktrees/*" >&2; \
		exit 1; \
	fi
	@echo "==> xtask verify-sources --offline（逐资产许可门禁）"
	@$(CARGO) run -p xtask -- verify-sources --offline
	@echo "==> xtask corpus-contract --scale $(CORPUS_GATE_SCALE)（建库后逐条跑契约）"
	@$(CARGO) run -p xtask -- corpus-contract --scale $(CORPUS_GATE_SCALE)
	@echo "==> xtask corpus-quality（含逐原因码基线漂移门禁）"
	@$(CARGO) run -p xtask -- corpus-quality
	@echo "==> 重新生成的语料报告必须与提交的版本逐字节一致"
	@git diff --exit-code -- corpus/reports/ || { \
		echo "corpus/reports/ 下的生成物与重新生成的结果不一致。" >&2; \
		echo "这些文件由 xtask 持有：跑 make corpus-gate 后把改动一起提交，不要手工编辑。" >&2; \
		exit 1; \
	}
	@echo "==> xtask commentary-index --check（集评出处索引漂移门禁）"
	@$(CARGO) run -p xtask -- commentary-index --check
	@echo "==> xtask corpus-measure --render-only（校验已提交的实测报告仍然通过门禁）"
	@# 刻意**不**在 CI 里重跑实测：`corpus-measure` 要求三个上游检出（合计约 830 MB），
	@# 而全量规模一次构建约 48 分钟。CI 里能验、且值得验的是另一件事——已提交的
	@# `measurements.json` 是否仍然解析得开并通过校验器（占位符、缺测量值、零值、
	@# 超预算不指名缓解措施都会失败）。这正是 todo 21 打包时要读的那份文件，所以
	@# 这一步守的是「结论文件没有腐坏」，而不是「数字是新的」。
	@$(CARGO) run -p xtask -- corpus-measure --render-only
	@echo "==> 黄金查询契约：fixture 自检 + 首启派生后的 37 条契约"
	@# 第二条是本 todo 引入的：随包工件不含 ngram / poem_fts / poem_last_char，
	@# 三者由首启在本机派生。这个测试先建出随包形态、再跑首启，然后逐条跑满契约——
	@# 它是「不随包不等于功能缩减」这句话的可执行形态，因此必须在门禁里。
	@$(CARGO) test -p yunjian-core --test golden_queries
	@$(CARGO) test -p yunjian-corpus --test first_launch_contracts
	@echo "==> 打包与派生的中止断言"
	@$(CARGO) test -p xtask corpus_package
	@$(CARGO) test -p yunjian-core --lib derive::
	@echo "语料门禁通过。"

# 发布语料工件的本机入口。刻意**不**属于 `ci`：它要三个上游检出（合计约 830 MB）、
# 一次约 11 分钟的建库和一次约 10 分钟的首启派生，而且它的产物不进版本库。
# CI 上同一组命令由 `.github/workflows/corpus-release.yml` 在 `corpus-v*` tag 上跑。
corpus-artifact: ## 构建并打包语料工件（需三个上游检出，约 25 分钟）
	@test -n "$(CHINESE_POETRY_DIR)" -a -n "$(WERNEROR_DIR)" -a -n "$(RHYME_DIR)" || { \
		echo "用法：make corpus-artifact CHINESE_POETRY_DIR=… WERNEROR_DIR=… RHYME_DIR=…" >&2; \
		echo "三个目录必须是按 corpus/sources.toml 锁定 revision 的检出。" >&2; \
		exit 1; \
	}
	@echo "==> xtask corpus-build（随包库 + 审计库，跨文件守恒在构建内校验）"
	@$(CARGO) run -p xtask --release -- corpus-build \
		--chinese-poetry-dir "$(CHINESE_POETRY_DIR)" \
		--werneror-dir "$(WERNEROR_DIR)" \
		--rhyme-dir "$(RHYME_DIR)"
	@echo "==> xtask corpus-package（六条中止断言 + 解压回读）"
	@$(CARGO) run -p xtask --release -- corpus-package
	@echo "==> 独立复核 sha256 与 manifest"
	@cd corpus/build/package && sha256sum -c ./*.db.gz.sha256
	@cd corpus/build/package && jq -e \
		'.schema_version and .corpus_version and .min_app_version and .sha256' \
		manifest.json > /dev/null
	@echo "语料工件就绪：corpus/build/package/"

hooks: ## 安装 git hooks：pre-commit 做格式化，pre-push 跑门禁
	@command -v $(PRE_COMMIT) >/dev/null 2>&1 || { \
		echo "缺少 pre-commit。安装：pip install pre-commit，或 mise use -g pre-commit" >&2; \
		exit 1; \
	}
	@$(PRE_COMMIT) install --hook-type pre-commit --hook-type pre-push
