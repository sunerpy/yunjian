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
GATE := fmt-check lint test mcp-conformance frontend-test pr-title-check

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

.PHONY: help fmt fmt-rust fmt-oxfmt fmt-check lint test mcp-conformance pr-title-check build check ci corpus-gate \
	corpus-artifact bundle clean-install hooks frontend frontend-test

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

mcp-conformance: ## 用真实 rmcp 客户端执行 MCP 端到端一致性测试
	@echo "==> cargo test -p yunjian-mcp --features http --test conformance"
	@$(CARGO) test -p yunjian-mcp --features http --test conformance

# 自检 PR 标题校验器本身，跑的是真实历史标题（含已经真的丢过版本的那 7 条）与合成用例。
# **在 GATE 里，不是可选补充**：这个校验器挡的是一类静默失败（标题不合规 → release-please
# 不报错也不 bump），而一个坏掉的校验器同样会静默放行。判定逻辑只有一份，
# `.github/workflows/pr-title.yml` 用的就是这个脚本，本地与 CI 不存在两套标准。
pr-title-check: ## 自检 PR 标题校验器（真实历史 + 合成用例）
	@echo "==> scripts/check-pr-title.py --self-test"
	@python3 scripts/check-pr-title.py --self-test

# 语音特性（`--features voice`）刻意不进任何门禁：它会拉起需要 libclang 与
# 网络下载的原生依赖，且分发物许可与默认构建不同。要验它请显式指定特性。
build: ## 发布构建。刻意不属于 ci：耗时长，且不是正确性门禁
	@echo "==> cargo build --release"
	@$(CARGO) build --release

# 桌面安装包的本机入口。**刻意不属于 `ci`**：一次 debug 打包约 12 分钟、铺开约 1.2 GiB
# 中间件，而 `ci` 是 pre-push 钩子跑的那一条。三件事在这里做，而不是留给裸
# `cargo tauri build`：
#
# 1. `--no-sign`。`tauri.conf.json` 声明了 `plugins.updater.pubkey`，于是打包最后一步
#    必然要签名，没有 `TAURI_SIGNING_PRIVATE_KEY` 就报「A public key has been found,
#    but no private key」并整体退出 1 —— 而**三个安装包此时其实已经全部产出**。这个失败
#    形态极易被读成「打包失败」，实际是「本机没有发布私钥」。本机与容器不该持有发布私钥，
#    所以本目标显式不签名；发布签名只在 `.github/workflows/release-please.yml` 里做，
#    且那边缺 key 是硬失败（`updater 签名不可关闭`），不会被本目标削弱。
# 2. `-v`。tauri-bundler 的 `log_level` 默认是 `Error`，那条分支用 `cmd.output()` 吞掉
#    linuxdeploy 的 stderr，只抛一句 `failed to run linuxdeploy`。加 `-v` 走
#    `output_ok()`，真实原因（缺库、磁盘满、插件失败）才进得了日志。
# 3. 打完逐类核对产物。`cargo tauri build` 少打一个安装包时确实会非零退出，但在有人手动
#    跑一次之前没有任何东西会发现；而 **Linux updater 只消费 AppImage（`.deb` 不能自动
#    更新）**，少这一个不是「少一个可选产物」，是断掉 Linux 的自动更新链。
BUNDLE_KINDS := deb rpm appimage
BUNDLE_DIR := target/debug/bundle
BUNDLE_BINARY := target/debug/yunjian-desktop
# AppImage 阶段要把整个 GTK/WebKit 栈复制进一个未压缩 AppDir 再压一遍 squashfs，debug
# 产物尤其大（实测 AppDir 588 MiB + appimage_deb 389 MiB + 输出 163 MiB）。磁盘不够时
# linuxdeploy 的失败同样只显示成那句 `failed to run linuxdeploy`，一个字都不提 ENOSPC，
# 所以先判空间，把一个已知会误导人的失败提前变成一句人话。
BUNDLE_MIN_FREE_MB := 4096

bundle: | $(FRONTEND_DIST) ## 打桌面安装包并逐类核对产物（deb / rpm / AppImage）
	@free=$$(df -Pm . | awk 'NR==2 {print $$4}'); \
	if [ "$$free" -lt $(BUNDLE_MIN_FREE_MB) ]; then \
		echo "可用磁盘 $${free} MiB，低于 $(BUNDLE_MIN_FREE_MB) MiB。" >&2; \
		echo "AppImage 阶段会先铺开一个未压缩 AppDir，空间不足时 linuxdeploy 的失败" >&2; \
		echo "只显示成 'failed to run linuxdeploy'，不会说是 ENOSPC。" >&2; \
		exit 1; \
	fi
	@echo "==> cargo tauri build --debug --no-sign -v"
	@cd crates/yunjian-app && $(CARGO) tauri build --debug --no-sign -v
	@test -x "$(BUNDLE_BINARY)" || { echo "缺少可执行产物 $(BUNDLE_BINARY)" >&2; exit 1; }
	@echo "==> 逐类核对产物：$(BUNDLE_KINDS)"
	@status=0; \
	printf '    %-9s %8s  %s\n' binary "$$(du -h "$(BUNDLE_BINARY)" | cut -f1)" "$(BUNDLE_BINARY)"; \
	for kind in $(BUNDLE_KINDS); do \
		case "$$kind" in \
			deb) pattern='*.deb' ;; \
			rpm) pattern='*.rpm' ;; \
			appimage) pattern='*.AppImage' ;; \
			*) echo "未知产物类别 $$kind：加类别时要同时给出它的文件名模式" >&2; exit 1 ;; \
		esac; \
		found=$$(find "$(BUNDLE_DIR)/$$kind" -maxdepth 1 -type f -name "$$pattern" 2>/dev/null | sort); \
		if [ -z "$$found" ]; then \
			echo "缺少 $$kind 产物（$(BUNDLE_DIR)/$$kind/$$pattern）" >&2; \
			status=1; \
			continue; \
		fi; \
		echo "$$found" | while read -r file; do \
			printf '    %-9s %8s  %s\n' "$$kind" "$$(du -h "$$file" | cut -f1)" "$$file"; \
		done; \
	done; \
	if [ "$$status" -ne 0 ]; then \
		echo "安装包不齐。Linux updater 只消费 AppImage（.deb 不能自动更新），" >&2; \
		echo "少 appimage 不是少一个可选产物，是断掉 Linux 的自动更新链。" >&2; \
		exit 1; \
	fi; \
	echo "安装包齐备：$(BUNDLE_KINDS)"

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

# 净机安装验收。刻意**不**属于 `ci`：它要 docker、一份已打包的语料工件、一次约 10 分钟的
# 首启派生，而且断网段必须是**另一个**容器（同一个容器不可能既能下载又没有网络）。
#
# 三段：联网容器跑安装与全流程 -> 断网容器（`--network none`）只跑字典命令 ->
# 宿主裁决并写报告。断言集在 `xtask clean-install-report` 里预声明，少交一条观测即中止。
#
# `MIRROR_BASE` 指向一个提供发布资产的 HTTP 前缀。用本地镜像而不是 GitHub 是刻意的：
# 验的是安装脚本与工件，不是 GitHub 的可用性；且切 tag 之前就能先验一遍。
#
# 净机镜像必须**自带** curl 或 wget：`install.sh` 二者都没有时在第一步就中止，而在容器里
# 装一个会让掉「净」这个性质（被我们改造过的容器验不了「用户在干净机器上装得上」）。
# 2026-08-18 逐个实测：`ubuntu:24.04` 与 `debian:12` 两者都没有；`fedora:41` 自带 curl
# （glibc 2.40）；`alpine:3.20` 自带 BusyBox wget（musl）。缺省取 fedora，
# 用 `CLEAN_INSTALL_IMAGE=alpine:3.20 CLEAN_INSTALL_SLUG=alpine` 再跑一遍验 musl 侧。
CLEAN_INSTALL_IMAGE ?= fedora:41
CLEAN_INSTALL_SLUG ?=
CLEAN_INSTALL_PROFILE := $(CURDIR)/target/clean-install-profile
# 观测目录按镜像分开。共用一个目录时后一个镜像的 `rm -f *.tsv` 会把前一个的观测删掉，
# 于是前一份报告再也重算不出来——同一天在两个净镜像上各跑一遍是常态，这个覆盖必然发生。
CLEAN_INSTALL_OBS := $(CURDIR)/target/clean-install-observed$(if $(CLEAN_INSTALL_SLUG),-$(CLEAN_INSTALL_SLUG),)

# 容器里要清掉的代理变量。docker CLI 会把 `~/.docker/config.json` 里的 `proxies.default`
# 注入**每个**容器，本机注的是 `http://127.0.0.1:1080`——那个地址在容器命名空间里指向容器
# 自己，于是 wget/curl 报「can't connect to remote host (127.0.0.1)」，读起来像本地镜像挂了。
# 清掉它是**移除宿主带进来的污染**而不是改造净机：一台真实的干净机器不会有一个指向不存在
# 代理的 http_proxy。`-e http_proxy=` 置空不够——实测 BusyBox wget 对空值仍走代理路径，
# 必须在容器内 `unset`。
CLEAN_INSTALL_UNSET_PROXY := unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY \
	all_proxy ALL_PROXY no_proxy NO_PROXY;

clean-install: ## 在净容器里验收安装、取数、离线可用与 provider 计数（需 docker + MIRROR_BASE）
	@test -n "$(MIRROR_BASE)" -a -n "$(ARTIFACTS_DIR)" || { \
		echo "用法：make clean-install MIRROR_BASE=http://<host>:<port> ARTIFACTS_DIR=<资产目录>" >&2; \
		echo "MIRROR_BASE 下需有 v<版本>/ 与 corpus-v<版本>/ 两个前缀；ARTIFACTS_DIR 是后者的本地路径。" >&2; \
		exit 1; \
	}
	@command -v docker >/dev/null 2>&1 || { echo "缺少 docker：净机验收无法在宿主上就地跑" >&2; exit 1; }
	@echo "==> provider 调用计数（宿主，fixture 种子）"
	@$(CARGO) run -p xtask --release -- provider-calls \
		--out docs/reports/clean-install-provider-calls.json
	@mkdir -p "$(CLEAN_INSTALL_OBS)"
	@rm -f "$(CLEAN_INSTALL_OBS)"/*.tsv
	@echo "==> 清空净机 profile（用容器清，里面的文件是 root 所有）"
	@mkdir -p "$(CLEAN_INSTALL_PROFILE)"
	@docker run --rm -v "$(CLEAN_INSTALL_PROFILE)":/p $(CLEAN_INSTALL_IMAGE) \
		sh -c 'rm -rf /p/* /p/.[!.]* 2>/dev/null; test "$$(ls -A /p | wc -l)" = 0'
	@echo "==> 探测净机自带的下载器（不安装任何东西）"
	@downloader=$$(docker run --rm $(CLEAN_INSTALL_IMAGE) \
		sh -c 'command -v curl || command -v wget || echo NONE'); \
	if [ "$$downloader" = NONE ]; then \
		echo "净机 $(CLEAN_INSTALL_IMAGE) 既无 curl 也无 wget：install.sh 会在第一步中止。" >&2; \
		echo "不要在容器里装一个——那让掉「净」这个性质。换一个自带下载器的镜像：" >&2; \
		echo "  make clean-install CLEAN_INSTALL_IMAGE=fedora:41 ...      # 自带 curl（glibc）" >&2; \
		echo "  make clean-install CLEAN_INSTALL_IMAGE=alpine:3.20 ...    # 自带 wget（musl）" >&2; \
		exit 1; \
	fi; \
	echo "$$downloader" > "$(CLEAN_INSTALL_OBS)/downloader.txt"; \
	echo "自带 $$downloader"
	@echo "==> 联网净机容器"
	@docker run --rm -v "$(CURDIR)":/work:ro -v "$(CLEAN_INSTALL_OBS)":/observed \
		-v "$(CLEAN_INSTALL_PROFILE)":/root \
		-e YUNJIAN_PHASE=online -e YUNJIAN_MIRROR_BASE="$(MIRROR_BASE)" \
		-e YUNJIAN_OBSERVED=/observed/online.tsv \
		$(CLEAN_INSTALL_IMAGE) sh -c '$(CLEAN_INSTALL_UNSET_PROXY) export HOME=/root; \
			sh /work/scripts/clean-install-verify.sh'
	@echo "==> 断网净机容器（--network none）"
	@docker run --rm --network none -v "$(CURDIR)":/work:ro -v "$(CLEAN_INSTALL_OBS)":/observed \
		-v "$(CLEAN_INSTALL_PROFILE)":/root \
		-e YUNJIAN_PHASE=offline -e YUNJIAN_MIRROR_BASE="$(MIRROR_BASE)" \
		-e YUNJIAN_OBSERVED=/observed/offline.tsv \
		$(CLEAN_INSTALL_IMAGE) sh -c '$(CLEAN_INSTALL_UNSET_PROXY) export HOME=/root; \
			sh /work/scripts/clean-install-verify.sh'
	@echo "==> 裁决并写报告"
	@$(CARGO) run -p xtask --release -- clean-install-report \
		--observed "$(CLEAN_INSTALL_OBS)/online.tsv" \
		--observed "$(CLEAN_INSTALL_OBS)/offline.tsv" \
		--artifacts-dir "$(ARTIFACTS_DIR)" \
		--image "$(CLEAN_INSTALL_IMAGE)" \
		--image-digest "$$(docker image inspect $(CLEAN_INSTALL_IMAGE) --format '{{.Id}}')" \
		--bundled-downloader "$$(cat "$(CLEAN_INSTALL_OBS)/downloader.txt")（镜像预装，未在容器里安装任何软件包）" \
		--os-release "$$(docker run --rm $(CLEAN_INSTALL_IMAGE) \
			sh -c '. /etc/os-release 2>/dev/null && echo "$$PRETTY_NAME" || echo unknown')" \
		--kernel "$$(docker run --rm $(CLEAN_INSTALL_IMAGE) uname -sr)" \
		--preexisting-home-entries 0 \
		--offline-isolation 'docker run --network none' \
		--date "$$(date -u +%Y-%m-%d)" \
		--slug "$(CLEAN_INSTALL_SLUG)" \
		--commit-sha "$$(git rev-parse HEAD)"

hooks: ## 安装 git hooks：pre-commit 做格式化，pre-push 跑门禁
	@command -v $(PRE_COMMIT) >/dev/null 2>&1 || { \
		echo "缺少 pre-commit。安装：pip install pre-commit，或 mise use -g pre-commit" >&2; \
		exit 1; \
	}
	@$(PRE_COMMIT) install --hook-type pre-commit --hook-type pre-push
