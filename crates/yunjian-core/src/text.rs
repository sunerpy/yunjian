//! 正文字符分类。
//!
//! 为什么这条规则必须只有一份实现：语料构建期用它数 `char_count`、取句尾字，
//! 运行期用它派生 1/2 字候选表（见 [`crate::derive`]）。两处一旦对「什么算正文字符」
//! 有分歧，首启构建出来的候选表就与语料库自己的计数对不上，而这种偏差不会报错，
//! 只会让某些查询少召回几条。定义放在运行时 crate 里，构建期 crate 反向引用它。

/// 判定一个字符是否为标点（ASCII 标点或中文常用标点）。
///
/// 不算正文的还有空白字符，见 [`content_chars`]。
#[must_use]
pub const fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '《'
                | '》'
                | '〈'
                | '〉'
                | '【'
                | '】'
                | '〔'
                | '〕'
                | '—'
                | '…'
                | '·'
        )
}

/// 逐字给出正文字符：去掉空白与标点，其余原样保留。
///
/// 换行也是空白，因此这个序列**跨句连续**——2 字候选跨句边界成立，是刻意的：
/// 用户查「明月」时不关心它是否恰好落在同一句里。
pub fn content_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_chars_drops_whitespace_and_both_punctuation_families() {
        let text = "床前明月光，\n疑是地上霜。";
        let content = content_chars(text).collect::<String>();
        assert_eq!(content, "床前明月光疑是地上霜");
    }

    #[test]
    fn ascii_and_chinese_punctuation_are_both_recognized() {
        for character in ['，', '。', '、', '《', '·', '…', ',', '.', '!', '-'] {
            assert!(is_punctuation(character), "{character} 应判为标点");
        }
        for character in ['床', '前', '明', 'a', '0'] {
            assert!(!is_punctuation(character), "{character} 不应判为标点");
        }
    }
}
