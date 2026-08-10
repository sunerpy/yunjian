//! 云笺统一错误类型。
//!
//! 全工作区共用 [`Error`] 与 [`Result`]，这样跨 crate 传播错误时不必层层包装。
//!
//! # 为什么 [`AiError`] 要单独存在
//!
//! MCP 客户端会把服务端的 stderr 整份录进日志文件（Claude Desktop、OpenCode 都如此），
//! 所以「密钥不出现在任何诊断里」不是风格问题，而是一旦破防就无法追回的泄露。
//! AI 供应商的错误文本天生带 URL、请求头与查询参数，是唯一有现实可能夹带凭据的路径，
//! 因此它的载荷类型 [`AiError`]：
//!
//! 1. 字段私有、构造入口只有 [`AiError::new`]，且在**构造时**就把凭据洗掉——
//!    密钥根本不会存在于结构体里，即使有人直接 `Debug` 打印内部字段也无从泄露；
//! 2. 手写 `Debug` / `Display`，渲染时再洗一遍（[`redact_credentials`] 幂等），
//!    这样即便将来有人绕过构造器塞进未净化的值，输出仍然是干净的。
//!
//! 其余变体不做净化：`Config` 与 `Io` 只承载路径（配置文件本身开了
//! `deny_unknown_fields`，密钥写进去会直接报错而不是被收下），`Corpus` / `Search` /
//! `Db` / `Voice` / `Recite` 都在离线数据路径上，没有凭据可携带。

use std::borrow::Cow;
use std::fmt;

/// 工作区统一的 `Result` 别名。
pub type Result<T> = std::result::Result<T, Error>;

/// 云笺的顶层错误类型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 语料库解析、物化或校验失败。
    #[error("语料库错误：{0}")]
    Corpus(String),

    /// 检索请求无法执行或结果不合法。
    #[error("检索错误：{0}")]
    Search(String),

    /// 配置发现、解析或校验失败。
    #[error("配置错误：{0}")]
    Config(String),

    /// 文件系统或其他 I/O 失败。
    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    /// SQLite 操作失败。
    #[error("数据库错误：{0}")]
    Db(#[from] rusqlite::Error),

    /// AI 供应商调用失败。载荷已脱敏，见 [`AiError`]。
    #[error("AI 错误：{0}")]
    Ai(AiError),

    /// 语音采集、合成或识别失败。
    #[error("语音错误：{0}")]
    Voice(String),

    /// 背诵对齐、评分或调度失败。
    #[error("背诵错误：{0}")]
    Recite(String),
}

impl Error {
    /// 构造一个已脱敏的 [`Error::Ai`]。
    pub fn ai(provider: impl Into<String>, detail: impl AsRef<str>) -> Self {
        Self::Ai(AiError::new(provider, detail))
    }
}

/// [`Error::Ai`] 的载荷。字段私有且在构造时脱敏，无法承载明文凭据。
#[derive(Clone, PartialEq, Eq)]
pub struct AiError {
    provider: String,
    detail: String,
}

impl AiError {
    /// 唯一构造入口。`detail` 在此处即被 [`redact_credentials`] 洗过，
    /// 因此结构体里永远不存在明文凭据。
    pub fn new(provider: impl Into<String>, detail: impl AsRef<str>) -> Self {
        Self {
            provider: provider.into(),
            detail: redact_credentials(detail.as_ref()).into_owned(),
        }
    }

    /// 供应商标识（如 `openai`）。不含凭据，故原样保留——诊断需要它才有意义。
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// 已脱敏的错误详情。
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for AiError {
    /// 手写而非 derive：渲染前再洗一遍，让净化不依赖「构造器一定被走过」这个前提。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "供应商 {} 调用失败：{}",
            self.provider,
            redact_credentials(&self.detail)
        )
    }
}

impl fmt::Debug for AiError {
    /// 手写而非 derive：derive 会把 `detail` 原样打出来，绕过 [`Display`] 的净化。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiError")
            .field("provider", &self.provider)
            .field("detail", &redact_credentials(&self.detail))
            .finish()
    }
}

/// 脱敏后填入的占位符。
const REDACTED: &str = "<已脱敏>";

/// 凭据前缀。命中其一且后随足够长的 token 串即整体脱敏。
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "pk-",
    "pk_",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xapp-",
    "AIza",
    "AKIA",
    "ASIA",
    "ya29.",
];

/// 键名一旦命中，其值整段脱敏（不看值长什么样）。
const CREDENTIAL_NAMES: &[&str] = &[
    "authorization",
    "client_secret",
    "access_token",
    "refresh_token",
    "accesstoken",
    "api_key",
    "api-key",
    "apikey",
    "password",
    "passwd",
    "secret",
    "token",
    "auth",
    "key",
    "pwd",
];

/// 跟在凭据键名之后的认证方案词。`authorization: Bearer xxx` 里要脱敏的是 `xxx`
/// 而非 `Bearer`。此处可以放宽到四个词，因为前面已有键名限定，不存在歧义。
const KEYED_AUTH_SCHEMES: &[&str] = &["Bearer", "Token", "Basic", "Bot"];

/// 可以在自由文本里独立识别的方案词，只有 `Bearer`。
///
/// 实测踩到过：把 `Token` / `Basic` 也放进来，「缺少 token 配置」会被洗成
/// 「缺少 token <已脱敏>」、「Basic 认证失败」会丢掉「认证失败」——这两个词在中文
/// 诊断里是常用词，而 `bearer` 不是。方案词的大小写不可靠（HTTP 头大小写不敏感），
/// 所以不能靠区分大小写来消歧，只能缩小词表。
const STANDALONE_AUTH_SCHEMES: &[&str] = &["Bearer"];

/// 前缀式凭据的最短主体长度。太短会把 `sk-1` 这类正常标识误判成密钥。
const MIN_SECRET_BODY: usize = 6;

/// 认证方案后凭据的最短长度。
const MIN_SCHEME_TOKEN: usize = 4;

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// token 起始位置的判定。`-` 算边界而 `_` 不算，因此 `--api-key` 能被识别成键名，
/// 而 `my_key` 不会被当成键名 `key`。
fn is_boundary_byte(b: u8) -> bool {
    !(b.is_ascii_alphanumeric() || b == b'_')
}

fn starts_with_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

/// 匹配 `sk-XXXXXX` 形态，返回要脱敏的字节数。
fn match_prefixed_secret(rest: &str) -> Option<usize> {
    let prefix = SECRET_PREFIXES.iter().find(|p| rest.starts_with(**p))?;
    let body = rest[prefix.len()..]
        .bytes()
        .take_while(|b| is_token_byte(*b))
        .count();
    (body >= MIN_SECRET_BODY).then_some(prefix.len() + body)
}

/// 匹配 `Bearer XXXX`，返回 `(保留的方案前缀长度, 总长度)`。
fn match_scheme_token(rest: &str) -> Option<(usize, usize)> {
    let bytes = rest.as_bytes();
    let scheme = STANDALONE_AUTH_SCHEMES
        .iter()
        .find(|s| starts_with_ignore_case(bytes, s.as_bytes()))?;
    let gap = rest[scheme.len()..]
        .bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count();
    if gap == 0 {
        return None;
    }
    let head = scheme.len() + gap;
    let body = rest[head..]
        .bytes()
        .take_while(|b| !b.is_ascii_whitespace() && !matches!(*b, b'"' | b'\'' | b',' | b')'))
        .count();
    (body >= MIN_SCHEME_TOKEN).then_some((head, head + body))
}

/// 匹配 `api_key=XXX` / `token: XXX` / `password="XXX"`，返回值的 `(起, 止)` 字节偏移。
///
/// `flag_style` 为真时（键名前紧邻 `-`，即 `--api-key XXX` 这种命令行形态）额外接受
/// 空白作为分隔符。仅限这一情形是刻意的：无条件接受空白会把「缺少 token 配置」里的
/// 「配置」也洗掉，把正常中文诊断毁成占位符。
fn match_named_credential(rest: &str, flag_style: bool) -> Option<(usize, usize)> {
    let bytes = rest.as_bytes();
    let name_len = CREDENTIAL_NAMES
        .iter()
        .filter(|n| starts_with_ignore_case(bytes, n.as_bytes()))
        .map(|n| n.len())
        .max()?;
    if bytes.get(name_len).is_some_and(|b| is_token_byte(*b)) {
        return None;
    }

    let skip_blanks = |mut p: usize| {
        while bytes.get(p).is_some_and(|b| *b == b' ' || *b == b'\t') {
            p += 1;
        }
        p
    };

    let mut p = skip_blanks(name_len);
    match bytes.get(p) {
        Some(b'=' | b':') => p += 1,
        _ if flag_style && p > name_len => {}
        _ => return None,
    }
    p = skip_blanks(p);

    let quote = match bytes.get(p) {
        Some(q @ (b'"' | b'\'')) => {
            let q = *q;
            p += 1;
            Some(q)
        }
        _ => None,
    };

    // 值本身是 `Bearer xxx` 时，前移到方案词之后，让脱敏落在真正的凭据上。
    if let Some(scheme) = KEYED_AUTH_SCHEMES
        .iter()
        .find(|s| starts_with_ignore_case(&bytes[p..], s.as_bytes()))
    {
        let gap = rest[p + scheme.len()..]
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        if gap > 0 {
            p += scheme.len() + gap;
        }
    }

    let start = p;
    while let Some(b) = bytes.get(p) {
        let stop = match quote {
            Some(q) => *b == q,
            None => {
                b.is_ascii_whitespace()
                    || matches!(*b, b'&' | b',' | b')' | b'}' | b';' | b'"' | b'\'')
            }
        };
        if stop {
            break;
        }
        p += 1;
    }
    (p > start).then_some((start, p))
}

/// 把文本里像凭据的片段替换成 [`REDACTED`]。
///
/// 幂等：对已脱敏的文本再调用一次不会有额外变化。宁可多脱敏也不漏——把一个正常标识
/// 洗成占位符只是让诊断少一点信息，把密钥写进日志则不可挽回。
///
/// 三条规则：前缀式凭据（`sk-…`、`AIza…`、`ghp_…`）、认证方案后的凭据
/// （`Bearer …`）、以及凭据类键名的值（`api_key=…`、`password: …`）。
/// 全部模式都是 ASCII，因此只在 ASCII 字节处起匹配即可保证不切碎多字节字符。
pub fn redact_credentials(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut copied = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if !bytes[i].is_ascii() || (i > 0 && !is_boundary_byte(bytes[i - 1])) {
            i += 1;
            continue;
        }

        let rest = &input[i..];
        let flag_style = i > 0 && bytes[i - 1] == b'-';
        let hit = match_prefixed_secret(rest)
            .map(|len| (0, len))
            .or_else(|| match_scheme_token(rest))
            .or_else(|| match_named_credential(rest, flag_style));

        match hit {
            Some((keep, end)) => {
                out.push_str(&input[copied..i + keep]);
                out.push_str(REDACTED);
                copied = i + end;
                i = copied;
            }
            None => i += 1,
        }
    }

    if copied == 0 {
        return Cow::Borrowed(input);
    }
    out.push_str(&input[copied..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 计划点名的用例：`Error::Ai` 携带假密钥 `sk-TESTKEY123` 时，
    /// `Display` 与 `Debug` 两种渲染都不得出现 `TESTKEY`。
    #[test]
    fn ai_error_never_renders_the_key() {
        let leaky = "POST https://api.openai.com/v1/chat?api_key=sk-TESTKEY123 \
                     -> 401 (Authorization: Bearer sk-TESTKEY123)";
        let err = Error::ai("openai", leaky);

        for rendered in [format!("{err}"), format!("{err:?}")] {
            assert!(
                !rendered.contains("TESTKEY"),
                "渲染结果泄露了密钥：{rendered}"
            );
            assert!(
                rendered.contains(REDACTED),
                "渲染结果没有脱敏占位符，说明净化根本没跑：{rendered}"
            );
        }
    }

    /// 密钥不只是「渲染时不显示」，而是根本没进结构体——否则任何直接读字段的
    /// 代码路径都是一个泄露点。
    #[test]
    fn ai_error_does_not_store_the_key() {
        let err = AiError::new("openai", "key=sk-TESTKEY123");
        assert!(!err.detail().contains("TESTKEY"));
        assert_eq!(err.provider(), "openai");
    }

    /// 供应商名与非凭据文本必须留下来，否则脱敏把诊断洗成了废纸。
    #[test]
    fn ai_error_keeps_the_diagnosable_part() {
        let err = Error::ai("openai", "429 Too Many Requests，请稍后重试");
        let rendered = format!("{err}");
        assert!(rendered.contains("openai"));
        assert!(rendered.contains("429 Too Many Requests"));
        assert!(rendered.contains("请稍后重试"));
    }

    #[test]
    fn redacts_prefixed_secrets() {
        for raw in [
            "sk-TESTKEY123",
            "sk_TESTKEY123",
            "AIzaTESTKEY123",
            "ghp_TESTKEY123",
            "ya29.TESTKEY123",
            "前缀 sk-TESTKEY123 后缀",
        ] {
            let got = redact_credentials(raw);
            assert!(!got.contains("TESTKEY"), "未脱敏：{raw} -> {got}");
        }
    }

    #[test]
    fn redacts_named_credentials_regardless_of_value_shape() {
        for raw in [
            "api_key=hunter2xyz",
            "api-key: hunter2xyz",
            "apikey=\"hunter2xyz\"",
            "password: hunter2xyz",
            "token=hunter2xyz&model=gpt",
            "--api-key hunter2xyz",
        ] {
            let got = redact_credentials(raw);
            assert!(!got.contains("hunter2xyz"), "未脱敏：{raw} -> {got}");
        }
    }

    #[test]
    fn redacts_scheme_tokens_but_keeps_the_scheme() {
        let got = redact_credentials("Authorization: Bearer hunter2xyz");
        assert!(!got.contains("hunter2xyz"), "{got}");
        assert!(
            got.contains("Bearer"),
            "方案词被一起洗掉了，诊断价值下降：{got}"
        );
    }

    /// 键名出现在普通中文诊断里时，后面的词不是值，不能当成凭据洗掉。
    /// 这是「空白仅在命令行形态下算分隔符」那个决定的回归护栏。
    #[test]
    fn keeps_prose_that_merely_mentions_a_credential_name() {
        for raw in [
            "缺少 token 配置",
            "key 不存在于钥匙串",
            "authorization 头缺失",
        ] {
            assert_eq!(redact_credentials(raw), raw, "误脱敏：{raw}");
        }
    }

    /// 短标识不该被误判成密钥，否则日志会被占位符淹没。
    #[test]
    fn keeps_short_identifiers() {
        for raw in ["sk-1", "任务 sk-ab", "risk-free"] {
            assert_eq!(redact_credentials(raw), raw, "误脱敏：{raw}");
        }
    }

    /// 无凭据时不分配新串，避免每条日志都付一次拷贝。
    #[test]
    fn borrows_when_nothing_to_redact() {
        assert!(matches!(
            redact_credentials("语料库文件缺失：corpus.db"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact_credentials("api_key=sk-TESTKEY123").into_owned();
        assert_eq!(redact_credentials(&once), once);
    }

    /// 多字节字符紧邻凭据时不得把字符切碎（切碎会 panic 或产出乱码）。
    #[test]
    fn handles_multibyte_neighbours() {
        let got = redact_credentials("密钥「sk-TESTKEY123」无效");
        assert!(!got.contains("TESTKEY"), "{got}");
        assert!(got.starts_with("密钥「"), "{got}");
        assert!(got.ends_with("」无效"), "{got}");
    }

    #[test]
    fn io_and_db_errors_convert_with_from() {
        let io: Error = std::io::Error::other("坏了").into();
        assert!(matches!(io, Error::Io(_)));
        assert!(format!("{io}").starts_with("I/O 错误："));

        let db: Error = rusqlite::Error::QueryReturnedNoRows.into();
        assert!(matches!(db, Error::Db(_)));
        assert!(format!("{db}").starts_with("数据库错误："));
    }

    #[test]
    fn every_variant_displays_with_its_domain_prefix() {
        let cases = [
            (Error::Corpus("x".into()), "语料库错误："),
            (Error::Search("x".into()), "检索错误："),
            (Error::Config("x".into()), "配置错误："),
            (Error::Voice("x".into()), "语音错误："),
            (Error::Recite("x".into()), "背诵错误："),
            (Error::ai("p", "x"), "AI 错误："),
        ];
        for (err, prefix) in cases {
            assert!(format!("{err}").starts_with(prefix), "{err}");
        }
    }

    #[test]
    fn result_alias_is_usable() {
        fn f(ok: bool) -> Result<u8> {
            if ok {
                Ok(1)
            } else {
                Err(Error::Search("空计划".into()))
            }
        }
        assert_eq!(f(true).unwrap(), 1);
        assert!(f(false).is_err());
    }
}
