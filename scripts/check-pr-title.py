#!/usr/bin/env python3
"""校验 PR 标题是否是本仓库可发版的 Conventional Commit。

## 这道门禁在挡什么

仓库用 squash 合并，squash 提交的**主题**就是 release-please 唯一能读到的东西。
主题不合 Conventional Commits 时，release-please **不报错、静默不 bump** ——
那个 PR 的全部改动对版本号隐形。

本仓库已经因此丢过 7 次（`scripts/pr-title-history.tsv` 里 8 条 FAIL 中来自
`main` 的那 7 条，逐条都是实质 fix）。所以这不是假想风险，是已发生的事实。

配套的仓库设置必须是 `squash_merge_commit_title = PR_TITLE`。**两件事缺一不可**：
- 只改设置不加门禁 → 把「取值来源随机」换成「标题写错就静默丢版本」。
  证据就在 fixture 里：`pr#27` 那条 PR 标题没有前缀，而当时的 `COMMIT_OR_PR_TITLE`
  恰好取了分支上那个带前缀的提交主题，把它救了回来。改成 PR_TITLE 之后，
  同一个标题就会变成第 8 次静默丢失 —— 除非有这道门禁把它拦下。
- 只加门禁不改设置 → 门禁校验的那个字符串根本不是最终会被解析的那个。

## 规则从哪来（刻意不在这里硬编码）

允许的 type 集合**从 `release-please-config.json` 的 `changelog-sections` 现读**，
不写第二份清单。这样「门禁放过一个 release-please 不认识的 type」在机制上不可能发生，
而不是靠谁记得同步两处。配置缺失或读不出 type 时本脚本硬失败，**不回退到内置清单** ——
一旦有回退，两份清单就又可以悄悄分叉了。

其余规则的取值都按**真实历史**校准过，不是照抄模板（详见各条注释）。
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RELEASE_PLEASE_CONFIG = REPO_ROOT / "release-please-config.json"
HISTORY_FIXTURE = Path(__file__).resolve().parent / "pr-title-history.tsv"

# 主题长度上限。**刻意取 72 而不是 50。**
# `commit-msg` 约定写的是「建议 ≤ 50 字，硬性不超过 72 字」，方案 `.omo/plans/yunjian.md`
# 只抄了那句建议值。而真实历史里最长的合规主题是 58 字
# （`fix(voice): 按裁决改 scoring_mode 取值域为 guided_practice 与 coverage_advisory`），
# 卡在 50 会把一个已经合进 main 的正当标题判红。门禁只拦硬性上限，建议值交给 review。
SUBJECT_MAX_CHARS = 72

# 主题里至少要有一个汉字。判的是「有没有」而不是「全都是」：真实主题普遍中英混排
# （`instanceof`、`UniFFI`、`CodeBuild`、`scoring_mode`……），要求纯中文会把全部历史判红。
# 反过来只要求「非 ASCII」又会放过 emoji 标题，所以按码位区间判汉字。
# 区间取 CJK 基本区 + 扩展 A + 兼容表意 + 扩展 B/C/D（诗词用字里确有生僻字）。
_CJK_RANGES = (
    (0x3400, 0x4DBF),  # 扩展 A
    (0x4E00, 0x9FFF),  # 基本区
    (0xF900, 0xFAFF),  # 兼容表意
    (0x20000, 0x2EBEF),  # 扩展 B 起
)

# 主题结尾不允许标点。真实历史 108 条无一违反，加这条零误伤。
_TRAILING_PUNCT = "。，、；：！？「」『』（）,.;:!?)]}"

# `<type>(<scope>)!?: <subject>`
#
# **scope 刻意只校验形状，不做闭集枚举。** 方案里那串 scope 是举例而非白名单：
# 真实历史用过 `acceptance`、`desktop`、`readme`、`ci` 四个不在那串里的 scope，
# 且四个 PR 都已正当合并。闭集会把它们判红 —— 那是门禁的错，不是标题的错。
# 而 scope 写错**不影响** release-please 的 bump 判定（它只看 type），
# 所以这里换来的安全边际本来就很小，不值那个误伤。
_HEADER = re.compile(
    r"""
    ^
    (?P<type>[a-z]+)
    (?: \( (?P<scope>[^()]*) \) )?
    (?P<breaking>!?)
    :
    (?P<sep>\ ?)
    (?P<subject>.*)
    $
    """,
    re.VERBOSE | re.DOTALL,
)

_SCOPE_SHAPE = re.compile(r"^[a-z0-9][a-z0-9-]*$")

# GitHub 在 squash 时自己会往主题末尾补 ` (#N)`。作者再手写一遍就会出现两个。
_TRAILING_PR_REF = re.compile(r"\(#\d+\)\s*$")

# release-please 自己那个发布 PR 的标题（形如 `chore(main): release 0.1.0`）。
#
# **这是唯一豁免「主题必须有汉字」的形态，而豁免的是形状不是作者。**
# 理由有两层：
# 1. 那个标题由 release-please 生成，作者改不动 —— 它每次更新发布 PR 都会重新写回。
#    对着一个改不动的字符串判红，等于让发布 PR 永远合不进去，而发布 PR 恰恰是
#    **必须能变绿**的那一个（`ci.yml` 顶上那段注释、`.oxfmtignore` 里 changelog 那条
#    都是同一个教训）。
# 2. 「主题用中文」是给人写的风格约束，为的是 changelog 读起来一致；而这条标题本身
#    就是 changelog 的产物，不是被收录进 changelog 的条目。
#
# 豁免刻意**不做成「跳过整个作业」或「按 actor 放行」**：那会把一整类真实错误一起放过。
# 这里仍然校验它是合法的 Conventional Commit（类型在集合内、范围形状合规、冒号后一个空格），
# 只放开中文主题这一条。形状对不上（例如 `chore(main): release notes`）照样判红。
_RELEASE_PLEASE_TITLE = re.compile(r"^chore\([^()]+\): release \d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?$")


def _is_cjk(ch: str) -> bool:
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in _CJK_RANGES)


def load_allowed_types(config_path: Path = RELEASE_PLEASE_CONFIG) -> list[str]:
    """从 release-please 配置现读允许的 type，读不出就硬失败。"""
    if not config_path.exists():
        raise SystemExit(
            f"读不到 {config_path}：允许的 type 集合只有这一个来源，"
            "本脚本刻意不内置备份清单（有备份就会与配置分叉）。"
        )
    try:
        config = json.loads(config_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{config_path} 不是合法 JSON：{exc}") from exc

    types: list[str] = []
    for pkg in (config.get("packages") or {}).values():
        for section in pkg.get("changelog-sections") or []:
            kind = section.get("type")
            if isinstance(kind, str) and kind and kind not in types:
                types.append(kind)
    if not types:
        raise SystemExit(
            f"{config_path} 的 changelog-sections 里没有任何 type，"
            "无法判定哪些 type 可发版。"
        )
    return types


def check(title: str, allowed_types: list[str]) -> list[str]:
    """返回违规原因列表；空列表表示通过。"""
    problems: list[str] = []

    if title != title.strip():
        problems.append("标题首尾有空白字符（squash 主题会原样带上）")
    title = title.strip()

    if not title:
        return ["标题为空"]

    match = _HEADER.match(title)
    if match is None:
        problems.append(
            "不是 Conventional Commit：缺少 `<类型>(<范围>): ` 前缀。"
            "缺前缀的标题 release-please 直接跳过，这个 PR 的改动**不会计入版本号**，"
            "而且不报错。"
        )
        return problems

    kind = match.group("type")
    scope = match.group("scope")
    subject = match.group("subject")

    if kind not in allowed_types:
        problems.append(
            f"类型 `{kind}` 不在 release-please 认识的集合里"
            f"（来自 release-please-config.json：{'、'.join(allowed_types)}）"
        )

    if scope is None:
        problems.append("缺少范围：本仓库约定范围是必填的，写成 `类型(范围): 主题`")
    elif scope == "":
        problems.append("范围是空括号 `()`；要么写上子系统名，要么整个去掉括号（但本仓库要求必填）")
    elif not _SCOPE_SHAPE.match(scope):
        problems.append(f"范围 `{scope}` 形状不合规：只允许小写英文、数字与连字符，且以字母或数字开头")

    if match.group("sep") != " ":
        problems.append("冒号后必须有且只有一个半角空格")

    if not subject:
        problems.append("冒号后没有主题")
        return problems

    if subject != subject.strip():
        problems.append("主题首尾有多余空白")

    subject = subject.strip()

    if not _RELEASE_PLEASE_TITLE.match(title) and not any(_is_cjk(ch) for ch in subject):
        problems.append(
            "主题里没有汉字：本仓库的提交主题用简体中文（可中英混排，"
            "但纯英文主题会让 changelog 中英夹杂）"
        )

    if len(subject) > SUBJECT_MAX_CHARS:
        problems.append(f"主题 {len(subject)} 字，超过硬性上限 {SUBJECT_MAX_CHARS} 字")

    if subject[-1] in _TRAILING_PUNCT:
        problems.append(f"主题结尾有标点 `{subject[-1]}`")

    if _TRAILING_PR_REF.search(subject):
        problems.append("主题末尾不要手写 `(#编号)`：GitHub 在 squash 时会自己补一个，写了会出现两个")

    for ch in subject:
        if unicodedata.category(ch) in {"Cc", "Cf"}:
            problems.append("主题含控制字符或不可见格式字符")
            break

    return problems


def _cmd_check(title: str) -> int:
    allowed = load_allowed_types()
    problems = check(title, allowed)
    if not problems:
        print(f"PR 标题合规：{title.strip()}")
        return 0
    print(f"PR 标题不合规：{title.strip()}", file=sys.stderr)
    for problem in problems:
        print(f"  · {problem}", file=sys.stderr)
    print("", file=sys.stderr)
    print("格式：`<类型>(<范围>): <中文主题>`，例如 `fix(app): 补齐 IPC 必需 Channel`", file=sys.stderr)
    print(f"可用类型：{'、'.join(allowed)}", file=sys.stderr)
    print("范围是必填的小写英文子系统名（corpus / core / app / mobile / release / ci ……）", file=sys.stderr)
    print("改完 PR 标题后本检查会自动重跑（本工作流监听 edited 事件）。", file=sys.stderr)
    return 1


# 合成用例。历史语料证明「合规的没被判红」，这一组证明「不合规的真被判红」——
# 历史里只出现过「整段缺前缀」一种错法，其余错法必须靠合成用例才有断言。
_SYNTHETIC: tuple[tuple[str, bool, str], ...] = (
    ("fix(app): 补齐 IPC 必需 Channel", True, "基准合规形态"),
    ("feat(app)!: 改掉 IPC 契约", True, "破坏性变更的 `!` 允许"),
    ("fix(pr-title): 修带连字符的范围", True, "范围允许连字符"),
    ("feat(app2): 修带数字的范围", True, "范围允许数字"),
    ("修复桌面验收假失败", False, "缺前缀——历史上真丢过版本的那种错法"),
    ("fix: 缺范围", False, "范围必填"),
    ("fix(): 空范围", False, "空括号不算范围"),
    ("fix(App): 范围有大写", False, "范围只允许小写"),
    ("fix(app cli): 范围有空格", False, "范围不允许空格"),
    ("wip(app): 类型不在集合里", False, "类型必须是 release-please 认识的"),
    ("fix(app):没有空格", False, "冒号后必须有空格"),
    ("fix(app):  两个空格", False, "冒号后只允许一个空格"),
    ("fix(app): ", False, "没有主题"),
    ("fix(app): fix the IPC channel contract", False, "纯英文主题"),
    ("fix(app): 修好了。", False, "主题结尾有标点"),
    ("fix(app): 修好了 (#123)", False, "主题手写了 PR 编号"),
    ("fix(app): 修" + "长" * SUBJECT_MAX_CHARS, False, f"主题超过 {SUBJECT_MAX_CHARS} 字"),
    ("fix(app): 修" + "长" * (SUBJECT_MAX_CHARS - 1), True, f"主题正好 {SUBJECT_MAX_CHARS} 字（边界之内）"),
    ("  fix(app): 首尾有空白  ", False, "首尾空白会原样进 squash 主题"),
    ("fix(app): 修\u200b好", False, "主题含零宽字符"),
    ("Revert \"fix(app): 补齐 IPC 必需 Channel\"", False, "GitHub 回滚按钮生成的标题需要作者改写"),
    ("chore(main): release 0.1.0", True, "release-please 自己那个改不动的发布 PR 标题"),
    ("chore(main): release 1.2.3-rc.1", True, "预发布版本号同形"),
    ("chore(main): release notes", False, "豁免锚在版本号形状上，不是「chore(main): release」开头就放行"),
    ("chore(main): release 0.1.0 now", False, "豁免要求整条标题严格同形"),
)


def _cmd_self_test() -> int:
    allowed = load_allowed_types()
    failures: list[str] = []
    history = 0

    if not HISTORY_FIXTURE.exists():
        print(f"读不到历史语料 {HISTORY_FIXTURE}", file=sys.stderr)
        return 1

    for lineno, raw in enumerate(HISTORY_FIXTURE.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        parts = raw.split("\t")
        if len(parts) != 3:
            failures.append(f"语料第 {lineno} 行不是三列：{raw!r}")
            continue
        expected, title, origin = parts
        if expected not in {"PASS", "FAIL"}:
            failures.append(f"语料第 {lineno} 行判定列既不是 PASS 也不是 FAIL：{expected!r}")
            continue
        history += 1
        problems = check(title, allowed)
        actual = "PASS" if not problems else "FAIL"
        if actual != expected:
            failures.append(
                f"历史语料 [{origin}] 期望 {expected} 实得 {actual}：{title}"
                + (f"（{problems[0]}）" if problems else "")
            )

    for title, should_pass, why in _SYNTHETIC:
        problems = check(title, allowed)
        if should_pass and problems:
            failures.append(f"合成用例应通过却被判红（{why}）：{title!r} → {problems}")
        if not should_pass and not problems:
            failures.append(f"合成用例应判红却通过了（{why}）：{title!r}")

    if failures:
        print("PR 标题校验器自检失败：", file=sys.stderr)
        for failure in failures:
            print(f"  · {failure}", file=sys.stderr)
        return 1

    print(
        f"PR 标题校验器自检通过：真实历史 {history} 条、合成用例 {len(_SYNTHETIC)} 条，"
        f"允许类型 {len(allowed)} 个（现读 release-please-config.json）"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="校验 PR 标题是否是可发版的 Conventional Commit")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--title", help="待校验的 PR 标题")
    group.add_argument(
        "--self-test",
        action="store_true",
        help="用真实历史标题与合成用例自检校验器本身",
    )
    args = parser.parse_args()
    return _cmd_self_test() if args.self_test else _cmd_check(args.title)


if __name__ == "__main__":
    sys.exit(main())
