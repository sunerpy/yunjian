/**
 * 两种高亮各自的纯函数。
 *
 * # 为什么是两种，而不是一种
 *
 * 它们的**证据来源完全不同**，混成一个会让界面在两种情形下宣称同样的确定性：
 *
 * - **karaoke 高亮**由示范音的拼接时间戳驱动。那些时刻是逐音步合成的算术结果，
 *   是确定值（`prosody::splice`）。所以它可以逐音步精确推进。
 * - **已匹配前缀高亮**由偏置假设驱动。偏置解码把诗文本当 hotwords，因此它**倾向于吐出
 *   原文，哪怕用户跳过了整句**（`recognize.rs` 的模块文档）。所以它只是一个「大概读到
 *   这里」的提示，绝不能当成完整度，也绝不参与评分。
 *
 * 本文件不算任何分数：一个是「时刻落在哪个区间」，一个是「两串的公共前缀有多长」。
 * 两者都是查找而不是评分。`__tests__/voiceHighlight.test.ts` 正反两向钉住这一点。
 */

/**
 * 参考文本与偏置转写的公共前缀长度，按**字**计。
 *
 * 只取前缀而不做编辑距离对齐：编辑距离会给出「漏了哪几个字」，而那正是 CER 77% 下不可
 * 报告的东西——报出来就是在一段噪声上宣称字级结论。前缀长度只支持一句话：「识别器目前
 * 认到这里」，那句话在偏置解码下也是成立的。
 *
 * 两侧都先剥掉非文字字符（标点、空白），与内核 `content_chars` 的口径一致：参考文本带
 * 标点而转写不带，不剥的话第一个逗号就会把前缀截断。
 */
export function matchedPrefixLength(reference: string, biased: string | null): number {
  if (biased === null) {
    return 0;
  }
  const left = contentChars(reference);
  const right = contentChars(biased);
  let matched = 0;
  while (matched < left.length && matched < right.length && left[matched] === right[matched]) {
    matched += 1;
  }
  return matched;
}

/**
 * 一行里参与朗读的字，即去掉标点与空白之后剩下的。
 *
 * 判据用 `\p{L}`（Unicode 字母类）而不是逐个列举标点：汉字、拉丁字母都在其中，而全角
 * 与半角标点、空白都不在。这与 `session.rs` 的 `SessionScript::from_poem` 用
 * `char::is_alphabetic` 切句是同一条口径。
 */
export function contentChars(text: string): string[] {
  return [...text].filter((character) => /\p{L}/u.test(character));
}
