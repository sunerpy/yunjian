#!/bin/sh
# 净机验收执行体（POSIX sh，在**容器内**运行）。
#
# 只做两件事：按顺序跑一遍真实用户路径，把每条**预声明断言**的观测结果写成一行 TSV。
# 裁决与报告渲染由宿主侧 `xtask clean-install-report` 负责——它持有断言集的唯一定义，
# 观测行少一条都会让报告生成失败。这条分工是刻意的：容器里没有 jq 也没有 Rust，
# 而「谁来判 PASS」不该取决于容器里恰好装了什么。
#
# 观测行格式（三列，制表符分隔）：
#
#   <断言 id>\t<PASS|FAIL|NOT EXECUTED>\t<依据>
#
# 依据里的制表符与换行必须先压成空格，否则一行观测会被读成两行。
#
# 环境变量：
#
#   YUNJIAN_MIRROR_BASE   本地镜像的下载前缀（宿主上的 HTTP 服务），例如
#                         http://172.17.0.1:18075。install.sh 与统一清单都指向它。
#   YUNJIAN_PHASE         `online` 走安装与取数；`offline` 只跑字典命令。
#   YUNJIAN_OBSERVED      观测行写到哪个文件。
#
# 退出码：脚本本身总是 0（除了用法错误）。**单条断言失败不中止**——一次跑完全部
# 断言比在第一条失败处停下更有用，否则每修一条都要重跑一遍容器。

set -u

: "${YUNJIAN_PHASE:?需要 YUNJIAN_PHASE=online|offline}"
: "${YUNJIAN_OBSERVED:?需要 YUNJIAN_OBSERVED=<观测输出路径>}"

BIN="${HOME}/.local/bin/yunjian"
export PATH="${HOME}/.local/bin:${PATH}"

: >"${YUNJIAN_OBSERVED}"

# 把依据压成单行并截断。制表符与换行换成空格，然后按**字符**（不是字节）截断。
#
# 必须按字符截断：`head -c` 会把一个多字节汉字切成两半，产生非法 UTF-8 字节，
# 于是整份观测文件不再是合法 UTF-8，宿主侧的 JSON 报告随之作废。已实测踩到。
flatten() {
  # `cut -c` 在部分实现里其实按字节切，所以后面再用 `iconv -c` 丢掉被切坏的残字节。
  # 两步都做才与实现无关；缺 iconv 时退化成不截断（宁可长，不要非法字节）。
  if command -v iconv >/dev/null 2>&1; then
    tr '\t\n' '  ' | sed 's/  */ /g; s/^ //; s/ $//' |
      cut -c1-320 | iconv -f UTF-8 -t UTF-8 -c
  else
    tr '\t\n' '  ' | sed 's/  */ /g; s/^ //; s/ $//'
  fi
}

observe() {
  observe_id="$1"
  observe_verdict="$2"
  observe_detail="$(printf '%s' "$3" | flatten)"
  printf '%s\t%s\t%s\n' "${observe_id}" "${observe_verdict}" "${observe_detail}" \
    >>"${YUNJIAN_OBSERVED}"
  printf '[%s] %s — %s\n' "${observe_verdict}" "${observe_id}" "${observe_detail}" >&2
}

# 已发布种子的文件名。与 `yunjian_core::assets::APPRECIATION_SEED_FILE_NAME` 一致。
SEED_BASENAME="appreciations.json"

# 从 stdin 的 JSON 里抽出某个键的全部 16 位十六进制取值，一行一个。
# 只用 sed + grep：净机上没有 jq，为验收去装一个会把「净」这件事让掉。
hex_ids() {
  sed "s/\"$1\" *: *\"\([0-9a-f]\{16\}\)\"/\n@@\1@@\n/g" |
    sed -n 's/^@@\([0-9a-f]\{16\}\)@@$/\1/p'
}

# `search 明月` 前 $1 条结果的 poem_id。
poem_ids() {
  "${BIN}" search 明月 --limit "$1" --json 2>/dev/null | hex_ids poem_id
}

first_poem_id() {
  poem_ids "$1" | head -1
}

# 跑一条命令，回显退出码与末尾输出。stdout 与 stderr 分开收：本项目的契约是
# 「结果只走 stdout、日志只走 stderr」，混在一起就验不了这件事。
LAST_OUT=""
LAST_ERR=""
LAST_CODE=0
run() {
  LAST_OUT="$(mktemp)"
  LAST_ERR="$(mktemp)"
  "$@" >"${LAST_OUT}" 2>"${LAST_ERR}"
  LAST_CODE=$?
  return 0
}

out_head() { flatten <"${LAST_OUT}"; }
err_head() { flatten <"${LAST_ERR}"; }

# ------------------------------------------------------------------ online 段

phase_online() {
  : "${YUNJIAN_MIRROR_BASE:?online 段需要 YUNJIAN_MIRROR_BASE}"

  # 1. 安装脚本。走本地镜像而不是 GitHub：验的是脚本与产物，不是 GitHub 的可用性。
  if YUNJIAN_VERSION=0.1.0 \
    YUNJIAN_BASE_URL="${YUNJIAN_MIRROR_BASE}" \
    YUNJIAN_INSTALL_DIR="${HOME}/.local/bin" \
    sh /work/scripts/install.sh >/tmp/install.log 2>&1; then
    # 判据是**跑得起来**，不是 `-x`。`-x` 只看权限位：2026-08-18 在 `alpine:3.20`
    # （musl）上实测，`install.sh` 回退到 gnu 归档后退出 0、装出的文件 `-x` 为真，
    # 而执行它得到 `sh: not found`（缺 glibc 的 ELF interpreter）。用 `-x` 判 PASS
    # 会把一次装不出可用产品的安装记成成功。
    #
    # 退出码必须取自 `${BIN}` 本身。写成 `if v="$("${BIN}" --version | head -1)"`
    # 会拿到 `head` 的退出码——它在被测文件根本跑不起来时照样是 0，于是那条
    # 「装出来但跑不起来」的失败会被这层管道吃掉，重新变成 PASS。
    if "${BIN}" --version >/tmp/version.log 2>&1; then
      observe install_script_installs PASS \
        "install.sh 退出 0，$(head -1 /tmp/version.log) 已装到 ${BIN} 且可执行"
    elif [ -e "${BIN}" ]; then
      observe install_script_installs FAIL \
        "install.sh 退出 0 并落下 ${BIN}，但执行它失败：$(flatten </tmp/version.log)"
    else
      observe install_script_installs FAIL "install.sh 退出 0 但 ${BIN} 不存在"
    fi
  else
    observe install_script_installs FAIL "install.sh 非零退出：$(tail -5 /tmp/install.log | flatten)"
  fi

  if ! "${BIN}" --version >/dev/null 2>&1; then
    observe search_before_fetch_exits_3 "NOT EXECUTED" "没有可执行文件，后续断言无法执行"
    observe corpus_fetch_downloads_both "NOT EXECUTED" "没有可执行文件"
    observe assets_status_reports_both "NOT EXECUTED" "没有可执行文件"
    observe search_returns_results "NOT EXECUTED" "没有可执行文件"
    observe recite_scores_round "NOT EXECUTED" "没有可执行文件"
    observe mcp_handshake_and_tools_list "NOT EXECUTED" "没有可执行文件"
    observe shipped_hit_without_key "NOT EXECUTED" "没有可执行文件"
    observe cold_poem_without_key_asks_for_config "NOT EXECUTED" "没有可执行文件"
    return 0
  fi

  # 2. 失败场景：还没取语料就检索。必须退出 3（数据不可用）并指名 `corpus fetch`，
  #    而不是退出 1（查过了没有）——把语料缺失读成「诗库里没有李白」是这条边界上最贵的错。
  run "${BIN}" search 明月
  if [ "${LAST_CODE}" -eq 3 ] && grep -q 'corpus fetch' "${LAST_ERR}" "${LAST_OUT}"; then
    observe search_before_fetch_exits_3 PASS \
      "退出码 3 且消息指名 corpus fetch：$(err_head | flatten)"
  else
    observe search_before_fetch_exits_3 FAIL \
      "退出码 ${LAST_CODE}，stderr=$(err_head | flatten) stdout=$(out_head | flatten)"
  fi

  # 3. 取两件工件。统一清单由环境变量指向本地镜像。
  run env YUNJIAN_ASSETS_MANIFEST="${YUNJIAN_MIRROR_BASE}/corpus-v0.1.0/assets_manifest.json" \
    "${BIN}" corpus fetch
  if [ "${LAST_CODE}" -eq 0 ]; then
    observe corpus_fetch_downloads_both PASS "corpus fetch 退出 0：$(out_head | flatten)"
  else
    observe corpus_fetch_downloads_both FAIL \
      "corpus fetch 退出 ${LAST_CODE}：$(err_head | flatten)"
  fi

  # 4. 两件工件的版本与记录数都要报得出来。
  run "${BIN}" assets status --json
  if [ "${LAST_CODE}" -eq 0 ]; then
    observe assets_status_reports_both PASS "assets status --json：$(out_head | flatten)"
  else
    observe assets_status_reports_both FAIL \
      "assets status 退出 ${LAST_CODE}：$(err_head | flatten)"
  fi

  # 5. 检索。
  run "${BIN}" search 明月 --limit 3 --json
  if [ "${LAST_CODE}" -eq 0 ]; then
    observe search_returns_results PASS "search 明月 退出 0：$(out_head | flatten)"
  else
    observe search_returns_results FAIL "search 退出 ${LAST_CODE}：$(err_head | flatten)"
  fi

  # 6. 背诵一轮。作答从 stdin 读，故必须真喂进去一段文本；喂空串等于验了一个空轮次。
  # 检索信封里的字段名是 `poem_id`（不是 `stable_id`——那是语料表里的列名）。
  POEM_ID="$(first_poem_id 1)"
  if [ -z "${POEM_ID}" ]; then
    observe recite_scores_round FAIL "从 search --json 里取不到 stable_id，无法发起背诵"
  else
    LAST_OUT="$(mktemp)"
    LAST_ERR="$(mktemp)"
    printf '床前明月光\n疑是地上霜\n举头望明月\n低头思故乡\n' |
      "${BIN}" recite "${POEM_ID}" --mode cloze --json >"${LAST_OUT}" 2>"${LAST_ERR}"
    LAST_CODE=$?
    if [ "${LAST_CODE}" -eq 0 ]; then
      observe recite_scores_round PASS \
        "recite ${POEM_ID} --mode cloze 退出 0 并给出评分：$(out_head | flatten)"
    else
      observe recite_scores_round FAIL \
        "recite ${POEM_ID} 退出 ${LAST_CODE}：$(err_head | flatten)"
    fi
  fi

  # 7. MCP 握手 + tools/list。手写两条 JSON-RPC 喂 stdio，不引任何客户端库：
  #    净机上不该为了验协议先装一个 SDK。
  MCP_IN="$(mktemp)"
  {
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"clean-install","version":"0"}}}'
    printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
  } >"${MCP_IN}"
  MCP_OUT="$(mktemp)"
  MCP_ERR="$(mktemp)"
  "${BIN}" mcp <"${MCP_IN}" >"${MCP_OUT}" 2>"${MCP_ERR}"
  MCP_CODE=$?
  if grep -q '"serverInfo"' "${MCP_OUT}" && grep -q '"tools"' "${MCP_OUT}"; then
    TOOLS="$(sed -n 's/.*"name":"\([a-z_]*\)".*/\1/p' "${MCP_OUT}" | tr '\n' ' ')"
    observe mcp_handshake_and_tools_list PASS \
      "initialize 回了 serverInfo，tools/list 回了工具表（退出码 ${MCP_CODE}）：${TOOLS}"
  else
    observe mcp_handshake_and_tools_list FAIL \
      "退出码 ${MCP_CODE}，stdout=$(flatten <"${MCP_OUT}" | flatten) stderr=$(flatten <"${MCP_ERR}" | flatten)"
  fi

  # 8. 没配 key 时的随包赏析路径。命令行没有 `appreciate` 子命令——赏析只在桌面端与
  #    MCP 上暴露——所以这条只能走 MCP 的 `appreciate_poem`。**这里只看有没有走到随包层**
  #    （`source == "shipped"`），不对正文下结论：待发布数据集当前每条正文是未生成标记，
  #    正文是否为模型输出由宿主侧另一条断言如实裁决。
  # 种子文件用的是语料的列名 `stable_id`；检索信封用的是 `poem_id`。两处不同名不是笔误。
  SEED_FILE="${HOME}/.local/share/yunjian/${SEED_BASENAME}"
  [ -f "${SEED_FILE}" ] || SEED_FILE="$(find "${HOME}" -name "${SEED_BASENAME}" 2>/dev/null | head -1)"
  SEED_ID="$(hex_ids stable_id <"${SEED_FILE:-/dev/null}" | head -1)"
  if [ -z "${SEED_ID}" ]; then
    observe shipped_hit_without_key FAIL "找不到已发布的种子文件，取不到随包集里的 stable_id"
  else
    MCP_IN2="$(mktemp)"
    {
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"clean-install","version":"0"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
      printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"appreciate_poem","arguments":{"poem_id":"%s"}}}\n' "${SEED_ID}"
    } >"${MCP_IN2}"
    MCP_OUT2="$(mktemp)"
    "${BIN}" mcp <"${MCP_IN2}" >"${MCP_OUT2}" 2>/dev/null
    if grep -q '"source":"shipped"' "${MCP_OUT2}"; then
      observe shipped_hit_without_key PASS \
        "未配置任何 key，appreciate_poem(${SEED_ID}) 返回 source=shipped：$(flatten <"${MCP_OUT2}" | flatten)"
    elif grep -q '"configuration_required"' "${MCP_OUT2}"; then
      observe shipped_hit_without_key FAIL \
        "随包集里的 ${SEED_ID} 没有命中随包层，返回了 configuration_required：$(flatten <"${MCP_OUT2}" | flatten)"
    else
      observe shipped_hit_without_key FAIL \
        "appreciate_poem 未返回可判读结果：$(flatten <"${MCP_OUT2}" | flatten)"
    fi
  fi

  # 9. 随包集之外的诗，在没有 key 时必须**如实要求配置**而不是报错或空转。
  #    这条与上一条一起才说明「无 key 可用」是设计而不是巧合。
  COLD_ID="$(poem_ids 5 | grep -v "^${SEED_ID}\$" | head -1)"
  if [ -z "${COLD_ID}" ]; then
    observe cold_poem_without_key_asks_for_config "NOT EXECUTED" \
      "检索结果里取不到一个不同于随包首 ${SEED_ID} 的 stable_id"
  else
    MCP_IN3="$(mktemp)"
    {
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"clean-install","version":"0"}}}'
      printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
      printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"appreciate_poem","arguments":{"poem_id":"%s"}}}\n' "${COLD_ID}"
    } >"${MCP_IN3}"
    MCP_OUT3="$(mktemp)"
    "${BIN}" mcp <"${MCP_IN3}" >"${MCP_OUT3}" 2>/dev/null
    if grep -q '"configuration_required"' "${MCP_OUT3}"; then
      observe cold_poem_without_key_asks_for_config PASS \
        "冷诗 ${COLD_ID} 无 key 时返回 configuration_required 并给出设置路径：$(flatten <"${MCP_OUT3}" | flatten)"
    else
      observe cold_poem_without_key_asks_for_config FAIL \
        "冷诗 ${COLD_ID} 无 key 时未返回 configuration_required：$(flatten <"${MCP_OUT3}" | flatten)"
    fi
  fi
}

# ----------------------------------------------------------------- offline 段

phase_offline() {
  if ! "${BIN}" --version >/dev/null 2>&1; then
    observe offline_no_network_proved "NOT EXECUTED" "没有可执行文件"
    observe offline_dictionary_commands "NOT EXECUTED" "没有可执行文件"
    return 0
  fi

  # 先证明这个容器真的没网。缺了这一步，「离线可用」只是「这次恰好没联网」。
  #
  # 对照实验刻意用**产品自己的 HTTP 客户端**（`corpus fetch` 总会先去读统一清单），
  # 而不是 curl 或 ping：
  #   - curl 未必在场（断网容器是另一个新容器，上一段装的 curl 在它的可写层里，不在数据卷里）。
  #     一个「没有 curl 所以连不上」的分支会让这条断言无条件 PASS，等于什么都没验。
  #   - ICMP 在容器里常被单独屏蔽，ping 不通证不了 TCP 出不去。
  # 用产品的客户端还多证一件事：跑离线路径的那个进程确实出不去网。
  : "${YUNJIAN_MIRROR_BASE:=http://172.17.0.1:18075}"
  IFACES="$(ls /sys/class/net | tr '\n' ' ')"
  ROUTES="$(ip route 2>/dev/null | wc -l)"
  run env YUNJIAN_ASSETS_MANIFEST="${YUNJIAN_MIRROR_BASE}/corpus-v0.1.0/assets_manifest.json" \
    "${BIN}" corpus fetch
  if [ "${LAST_CODE}" -eq 0 ]; then
    observe offline_no_network_proved FAIL \
      "对照实验失败：断网容器里 corpus fetch 仍然读到了 ${YUNJIAN_MIRROR_BASE} 的清单，本段的离线结论不成立"
  else
    observe offline_no_network_proved PASS \
      "对照实验：用产品自己的 HTTP 客户端访问 ${YUNJIAN_MIRROR_BASE} 失败（corpus fetch 退出 ${LAST_CODE}：$(err_head)）；网络接口只有「${IFACES}」，路由表 ${ROUTES} 条"
  fi

  # 字典命令逐条跑。任一非零即失败，并把哪一条记进依据。
  OFFLINE_FAIL=""
  OFFLINE_OK=""
  for probe in \
    "search 明月 --limit 3" \
    "author 李白" \
    "rhyme 七阳 --book pingshui" \
    "corpus status"; do
    # shellcheck disable=SC2086
    run "${BIN}" ${probe}
    if [ "${LAST_CODE}" -eq 0 ]; then
      OFFLINE_OK="${OFFLINE_OK}[${probe}=0]"
    else
      OFFLINE_FAIL="${OFFLINE_FAIL}[${probe}=${LAST_CODE}: $(err_head | flatten)]"
    fi
  done

  # `show` 需要一个真实 id，单独取。
  SHOW_ID="$(first_poem_id 1)"
  if [ -n "${SHOW_ID}" ]; then
    run "${BIN}" show "${SHOW_ID}" --json
    if [ "${LAST_CODE}" -eq 0 ]; then
      OFFLINE_OK="${OFFLINE_OK}[show ${SHOW_ID}=0]"
    else
      OFFLINE_FAIL="${OFFLINE_FAIL}[show ${SHOW_ID}=${LAST_CODE}]"
    fi
  else
    OFFLINE_FAIL="${OFFLINE_FAIL}[show=取不到 stable_id]"
  fi

  if [ -z "${OFFLINE_FAIL}" ]; then
    observe offline_dictionary_commands PASS "断网下全部退出 0：${OFFLINE_OK}"
  else
    observe offline_dictionary_commands FAIL "断网下有命令失败：${OFFLINE_FAIL}"
  fi
}

case "${YUNJIAN_PHASE}" in
  online) phase_online ;;
  offline) phase_offline ;;
  *)
    printf 'error: YUNJIAN_PHASE 只接受 online 或 offline\n' >&2
    exit 2
    ;;
esac

exit 0
