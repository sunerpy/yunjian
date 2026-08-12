//! 保留字节偏移的 JSONC 扫描器，以及基于字节区间的成员拼接。
//!
//! # 为什么不能「反序列化再序列化」
//!
//! 写客户端配置的硬要求是**无关条目在合并后逐字节不变**。把文件读成
//! `serde_json::Value` 再 `to_string_pretty` 写回，会顺手改掉用户的缩进、键序与
//! 空行，还会把注释全部吃掉——那不是合并，是重排。因此本模块的做法是**定位 +
//! 文本拼接**：只替换 `yunjian` 那一段字节，其余原文照抄。
//!
//! # 为什么注释处理不能只看扩展名
//!
//! 实测一份真实的 OpenCode 用户配置：文件名是 `opencode.json`，第 89 行却有一条
//! `//` 注释，`json.load` 当场报 `Expecting value: line 89 column 5`。客户端接受
//! JSONC 而不在乎扩展名，所以[`strip_comments`] 对所有目标文件一律执行，
//! `.jsonc` 不是开关。
//!
//! # 偏移为什么必须守住
//!
//! [`strip_comments`] 把注释里的每个**字节**换成一个空格（换行原样保留），
//! 于是剥离后的文本与原文长度逐字节对应：在剥离文本上扫出来的区间可以直接拿去
//! 切原文。若改成删除注释，所有偏移都会漂移，而漂移的后果是切错位置——把用户的
//! 别的条目截掉一半，正是本模块要防的事。

use serde_json::Value;

/// 一个 JSON 对象在文本里的字节区间，含两端花括号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSpan {
    /// `{` 的字节位置。
    pub start: usize,
    /// `}` 之后的字节位置。
    pub end: usize,
}

/// 一个对象成员在文本里的字节区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberSpan {
    /// 键的开引号位置。
    pub start: usize,
    /// 值末字节之后的位置。
    pub end: usize,
    /// 值的起始位置。
    pub value_start: usize,
    /// 值末字节之后的位置，与 [`Self::end`] 同值，独立命名是为了让调用点自证意图。
    pub value_end: usize,
}

/// 把 JSONC 注释替换成等长空格，字节偏移与行号双双保持不变。
///
/// 换行不替换成空格：块注释跨行时若把 `\n` 也换掉，`serde_json` 报错里的行号就会
/// 与用户看到的文件对不上。字符串字面量里的 `//` 与 `/*` 不是注释，转义 `\"` 不结束
/// 字符串——两条都由状态机处理。
///
/// 未闭合的块注释或字符串不在这里报错：剥离后的文本交给 `serde_json` 解析，由它给出
/// 带位置的诊断，比本函数自己造一份更有用。
#[must_use]
pub fn strip_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let end = string_end(bytes, index);
                out.extend_from_slice(&bytes[index..end]);
                index = end;
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    out.push(b' ');
                    index += 1;
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                out.extend_from_slice(b"  ");
                index += 2;
                while index < bytes.len() {
                    if bytes[index] == b'*' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                        out.extend_from_slice(b"  ");
                        index += 2;
                        break;
                    }
                    // 换行原样留下，其余字节（含多字节字符的每一个字节）各换一个空格。
                    out.push(if bytes[index] == b'\n' { b'\n' } else { b' ' });
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    // 只把注释字节换成 ASCII 空格与换行，字符串字面量原样保留，因此仍是合法 UTF-8。
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// 定位最外层 JSON 对象。文档不是对象时返回 `None`。
#[must_use]
pub fn root_object(stripped: &str) -> Option<ObjectSpan> {
    let bytes = stripped.as_bytes();
    let start = skip_whitespace(bytes, 0);
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let end = value_end(bytes, start)?;
    Some(ObjectSpan { start, end })
}

/// 在对象的**直接**成员里找指定键。
///
/// 只看直接成员：嵌套对象里同名的键与这里要找的不是一个东西，递归查找会把
/// `provider.mcp` 之类的巧合当成目标容器。
#[must_use]
pub fn find_member(stripped: &str, object: ObjectSpan, key: &str) -> Option<MemberSpan> {
    members(stripped, object)
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, span)| span)
}

/// 按出现顺序列出对象的直接成员。
///
/// 遇到不合法的结构就停下并返回已解析到的部分：调用方在此之前已经用
/// `serde_json` 验过文档，所以这里的提前结束只可能来自本模块自身的扫描缺陷，
/// 让它退化成「找不到成员」比 panic 安全——最坏结果是走到新建分支，而不是崩掉。
#[must_use]
pub fn members(stripped: &str, object: ObjectSpan) -> Vec<(String, MemberSpan)> {
    let bytes = stripped.as_bytes();
    let mut found = Vec::new();
    let mut index = skip_whitespace(bytes, object.start + 1);
    while index < object.end.saturating_sub(1) {
        if bytes.get(index) == Some(&b'}') {
            break;
        }
        if bytes.get(index) != Some(&b'"') {
            break;
        }
        let key_start = index;
        let key_end = string_end(bytes, key_start);
        let Some(name) = decode_string(&stripped[key_start..key_end]) else {
            break;
        };
        index = skip_whitespace(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            break;
        }
        let value_start = skip_whitespace(bytes, index + 1);
        let Some(value_end) = value_end(bytes, value_start) else {
            break;
        };
        found.push((
            name,
            MemberSpan {
                start: key_start,
                end: value_end,
                value_start,
                value_end,
            },
        ));
        index = skip_whitespace(bytes, value_end);
        if bytes.get(index) == Some(&b',') {
            index = skip_whitespace(bytes, index + 1);
        }
    }
    found
}

/// 该值是不是一个对象。
#[must_use]
pub fn is_object(stripped: &str, at: usize) -> bool {
    stripped.as_bytes().get(at) == Some(&b'{')
}

/// 把一个成员写进对象：已存在则只换值，不存在则插在末尾。
///
/// 返回**新的完整文本**。除被替换或插入的那一段，其余字节逐字来自 `original`
/// ——这正是「无关条目逐字节不变」这条要求在实现上的落点。
///
/// `original` 与 `stripped` 必须是同一份文本的两种形态（见 [`strip_comments`]），
/// 长度相同，否则区间会切错位置。
#[must_use]
pub fn upsert_member(
    original: &str,
    stripped: &str,
    object: ObjectSpan,
    key: &str,
    value: &Value,
) -> String {
    debug_assert_eq!(
        original.len(),
        stripped.len(),
        "剥离注释不得改变长度，否则字节区间无法复用"
    );
    let existing = members(stripped, object);
    let indent = member_indent(original, &existing, object);
    let rendered = render(value, &indent);

    if let Some((_, span)) = existing.iter().find(|(name, _)| name == key) {
        let mut out = String::with_capacity(original.len() + rendered.len());
        out.push_str(&original[..span.value_start]);
        out.push_str(&rendered);
        out.push_str(&original[span.value_end..]);
        return out;
    }

    let member = format!("{}: {rendered}", encode_key(key));
    match existing.last() {
        // 空对象：连同两端花括号一起重写成带一个成员的形态。
        None => {
            let closing = closing_indent(original, object);
            let mut out = String::with_capacity(original.len() + member.len() + 8);
            out.push_str(&original[..object.start]);
            out.push_str("{\n");
            out.push_str(&indent);
            out.push_str(&member);
            out.push('\n');
            out.push_str(&closing);
            out.push('}');
            out.push_str(&original[object.end..]);
            out
        }
        // 非空：紧跟在最后一个成员之后补 `,` 与新成员，不动它前面的任何字节。
        Some((_, last)) => {
            let mut out = String::with_capacity(original.len() + member.len() + 8);
            out.push_str(&original[..last.end]);
            out.push_str(",\n");
            out.push_str(&indent);
            out.push_str(&member);
            out.push_str(&original[last.end..]);
            out
        }
    }
}

/// 把值渲染成缩进到 `indent` 层级的 JSON 文本。
fn render(value: &Value, indent: &str) -> String {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| String::from("{}"));
    let mut lines = pretty.lines();
    let mut out = String::with_capacity(pretty.len() + indent.len() * 8);
    if let Some(first) = lines.next() {
        out.push_str(first);
    }
    for line in lines {
        out.push('\n');
        out.push_str(indent);
        out.push_str(line);
    }
    out
}

/// 新成员该用的缩进：抄第一个既有成员的，没有成员就在闭合花括号的缩进上加两格。
fn member_indent(original: &str, existing: &[(String, MemberSpan)], object: ObjectSpan) -> String {
    existing
        .first()
        .map(|(_, span)| line_indent(original, span.start))
        .unwrap_or_else(|| format!("{}  ", closing_indent(original, object)))
}

/// 闭合花括号所在行的缩进。
fn closing_indent(original: &str, object: ObjectSpan) -> String {
    line_indent(original, object.end.saturating_sub(1))
}

/// `at` 所在行行首到第一个非空白字符之间的空白。
fn line_indent(text: &str, at: usize) -> String {
    let line_start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    text[line_start..at]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// 把键写成合法的 JSON 字符串字面量。
fn encode_key(key: &str) -> String {
    Value::String(key.to_owned()).to_string()
}

/// 解码一个 JSON 字符串字面量（含两端引号）。
fn decode_string(literal: &str) -> Option<String> {
    serde_json::from_str(literal).ok()
}

/// 跳过空白，返回第一个非空白字节的位置。
fn skip_whitespace(bytes: &[u8], from: usize) -> usize {
    let mut index = from;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

/// 字符串字面量的结束位置（闭引号之后）。`at` 必须指向开引号。
fn string_end(bytes: &[u8], at: usize) -> usize {
    let mut index = at + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// 一个值的结束位置（末字节之后）。
///
/// 括号配对必须是字符串感知的：`{"a": "}"}` 里那个 `}` 在字符串里，按裸字节配对会
/// 提前收尾，于是后面所有区间都错位。
fn value_end(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'"' => Some(string_end(bytes, at)),
        open @ (b'{' | b'[') => {
            let close = if *open == b'{' { b'}' } else { b']' };
            let mut depth = 0_usize;
            let mut index = at;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => {
                        index = string_end(bytes, index);
                        continue;
                    }
                    byte if byte == *open => depth += 1,
                    byte if byte == close => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    // 嵌套的另一种括号交给自己的配对循环，这里只需跳过它整段。
                    b'{' | b'[' => {
                        index = value_end(bytes, index)?;
                        continue;
                    }
                    _ => {}
                }
                index += 1;
            }
            None
        }
        _ => {
            let mut index = at;
            while index < bytes.len()
                && !matches!(bytes[index], b',' | b'}' | b']')
                && !bytes[index].is_ascii_whitespace()
            {
                index += 1;
            }
            (index > at).then_some(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ObjectSpan, find_member, is_object, members, root_object, strip_comments, upsert_member,
    };
    use serde_json::{Value, json};

    fn parsed(text: &str) -> (String, ObjectSpan) {
        let stripped = strip_comments(text);
        let object = root_object(&stripped).expect("根应当是对象");
        (stripped, object)
    }

    #[test]
    fn stripping_comments_preserves_every_byte_offset() {
        let text = "{\n  // 说明\n  \"a\": 1\n}";
        let stripped = strip_comments(text);
        assert_eq!(stripped.len(), text.len(), "剥离后长度必须不变");
        assert!(!stripped.contains("说明"), "注释内容必须被清掉：{stripped}");
        // 键的位置一字节没动，这是复用区间的全部前提。
        assert_eq!(stripped.find("\"a\""), text.find("\"a\""));
        serde_json::from_str::<Value>(&stripped).expect("剥离后应当是合法 JSON");
    }

    #[test]
    fn block_comments_keep_their_newlines_so_error_line_numbers_stay_true() {
        let text = "{\n/* 一\n   二 */\n  \"a\": 1\n}";
        let stripped = strip_comments(text);
        assert_eq!(stripped.len(), text.len());
        assert_eq!(
            stripped.matches('\n').count(),
            text.matches('\n').count(),
            "块注释里的换行必须留着：{stripped}"
        );
        serde_json::from_str::<Value>(&stripped).expect("剥离后应当是合法 JSON");
    }

    #[test]
    fn slashes_inside_strings_are_not_comments() {
        // 这是最容易写错的一处：把 URL 里的 `//` 当注释会把值截断成 `"https:`。
        let text = r#"{"$schema": "https://opencode.ai/config.json", "b": "/* 不是注释 */"}"#;
        let stripped = strip_comments(text);
        assert_eq!(stripped, text, "字符串里的斜杠一个都不能动");
        let value: Value = serde_json::from_str(&stripped).expect("合法 JSON");
        assert_eq!(value["$schema"], json!("https://opencode.ai/config.json"));
    }

    #[test]
    fn escaped_quotes_do_not_end_the_string() {
        let text = r#"{"a": "他说\"//\"不是注释"}"#;
        assert_eq!(strip_comments(text), text);
    }

    #[test]
    fn a_multibyte_comment_is_replaced_byte_for_byte() {
        // 一个汉字三字节，若按「字符换一个空格」处理，长度会缩短、偏移随之漂移。
        let text = "{\n  //中文注释\n  \"a\": 1\n}";
        let stripped = strip_comments(text);
        assert_eq!(stripped.len(), text.len());
    }

    #[test]
    fn members_are_located_in_source_order() {
        let (stripped, object) = parsed("{\n  \"a\": 1,\n  \"b\": {\"c\": 2}\n}");
        let names: Vec<String> = members(&stripped, object)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn a_brace_inside_a_string_does_not_end_the_object() {
        let text = r#"{"a": {"x": "}"}, "b": 1}"#;
        let (stripped, object) = parsed(text);
        let names: Vec<String> = members(&stripped, object)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            vec!["a".to_owned(), "b".to_owned()],
            "字符串里的花括号让配对提前收尾时，b 会消失"
        );
    }

    #[test]
    fn nested_arrays_and_objects_are_skipped_whole() {
        let text = r#"{"a": [1, {"b": [2, 3]}, 4], "c": 5}"#;
        let (stripped, object) = parsed(text);
        let found = members(&stripped, object);
        assert_eq!(found.len(), 2);
        let (_, span) = &found[0];
        assert_eq!(
            &stripped[span.value_start..span.value_end],
            r#"[1, {"b": [2, 3]}, 4]"#
        );
    }

    #[test]
    fn only_direct_members_are_found() {
        // 嵌套里的 `mcp` 不是顶层容器，认错它会把服务器条目写进别人的子树。
        let text = r#"{"provider": {"mcp": {"x": 1}}, "theme": "system"}"#;
        let (stripped, object) = parsed(text);
        assert!(find_member(&stripped, object, "mcp").is_none());
        assert!(find_member(&stripped, object, "provider").is_some());
    }

    #[test]
    fn upserting_into_a_populated_object_leaves_the_other_members_byte_identical() {
        let text = "{\n  \"mcp\": {\n    \"other\": {\n      \"type\": \"local\"\n    }\n  }\n}";
        let (stripped, root) = parsed(text);
        let container = find_member(&stripped, root, "mcp").expect("mcp 应当存在");
        assert!(is_object(&stripped, container.value_start));
        let inner = ObjectSpan {
            start: container.value_start,
            end: container.value_end,
        };
        let updated = upsert_member(text, &stripped, inner, "yunjian", &json!({"a": 1}));
        let value: Value = serde_json::from_str(&updated).expect("结果必须是合法 JSON");
        assert_eq!(value["mcp"]["other"]["type"], json!("local"));
        assert_eq!(value["mcp"]["yunjian"]["a"], json!(1));
        // 原文里那段无关条目在结果里必须逐字出现。
        assert!(
            updated.contains("\"other\": {\n      \"type\": \"local\"\n    }"),
            "无关条目被重排了：{updated}"
        );
    }

    #[test]
    fn upserting_an_existing_member_replaces_only_its_value() {
        let text = "{\n  \"yunjian\": {\n    \"stale\": true\n  },\n  \"keep\": 1\n}";
        let (stripped, root) = parsed(text);
        let updated = upsert_member(text, &stripped, root, "yunjian", &json!({"fresh": true}));
        let value: Value = serde_json::from_str(&updated).expect("合法 JSON");
        assert_eq!(value["yunjian"], json!({"fresh": true}));
        assert_eq!(value["keep"], json!(1));
        assert!(!updated.contains("stale"), "旧值应当被换掉：{updated}");
    }

    #[test]
    fn upserting_into_an_empty_object_produces_a_single_member() {
        let text = "{\n  \"mcp\": {}\n}";
        let (stripped, root) = parsed(text);
        let container = find_member(&stripped, root, "mcp").expect("mcp 存在");
        let inner = ObjectSpan {
            start: container.value_start,
            end: container.value_end,
        };
        let updated = upsert_member(text, &stripped, inner, "yunjian", &json!({"a": 1}));
        let value: Value = serde_json::from_str(&updated).expect("合法 JSON");
        assert_eq!(value["mcp"]["yunjian"]["a"], json!(1));
    }

    #[test]
    fn comments_survive_the_upsert() {
        let text = "{\n  // 顶层说明\n  \"mcp\": {\n    // 条目说明\n    \"other\": {}\n  }\n}";
        let (stripped, root) = parsed(text);
        let container = find_member(&stripped, root, "mcp").expect("mcp 存在");
        let inner = ObjectSpan {
            start: container.value_start,
            end: container.value_end,
        };
        let updated = upsert_member(text, &stripped, inner, "yunjian", &json!({"a": 1}));
        assert!(updated.contains("// 顶层说明"), "顶层注释丢了：{updated}");
        assert!(updated.contains("// 条目说明"), "条目注释丢了：{updated}");
        serde_json::from_str::<Value>(&strip_comments(&updated)).expect("结果剥离注释后仍合法");
    }

    #[test]
    fn a_non_object_root_is_reported_rather_than_guessed() {
        assert!(root_object(&strip_comments("[1, 2]")).is_none());
        assert!(root_object(&strip_comments("\"text\"")).is_none());
        assert!(root_object(&strip_comments("")).is_none());
    }
}
