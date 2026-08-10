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
GATE := fmt-check lint test

# 语料门禁跑的样本规模。10k 是方案为 CI 指定的规模：足够大到让索引行为与真实语料同形
# （两字查询在 19 首上无论怎么查都是零点几毫秒，看不出路径退化），又足够小到能在几秒内跑完。
CORPUS_GATE_SCALE := 10000

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-rust fmt-oxfmt fmt-check lint test build check ci corpus-gate hooks

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
	@count=$$(find . -name queries.toml | wc -l); \
	if [ "$$count" -ne 1 ]; then \
		echo "仓库里有 $$count 份 queries.toml。契约只有一处，六方共同消费；" >&2; \
		echo "消费方请引用 crates/yunjian-core/tests/queries.toml，不要复制。" >&2; \
		find . -name queries.toml >&2; \
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
	@echo "==> 黄金查询契约：fixture 自检 + FTS5 索引回归"
	@$(CARGO) test -p yunjian-core --test golden_queries
	@$(CARGO) test -p yunjian-corpus fts::
	@echo "语料门禁通过。"

hooks: ## 安装 git hooks：pre-commit 做格式化，pre-push 跑门禁
	@command -v $(PRE_COMMIT) >/dev/null 2>&1 || { \
		echo "缺少 pre-commit。安装：pip install pre-commit，或 mise use -g pre-commit" >&2; \
		exit 1; \
	}
	@$(PRE_COMMIT) install --hook-type pre-commit --hook-type pre-push
