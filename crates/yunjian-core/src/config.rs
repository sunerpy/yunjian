//! 配置加载：把用户可手改的 `config.toml` 解析成类型化结构，全进程只解析一次。
//!
//! 采用「运行时发现」模式而非把配置编译进二进制：云笺是面向用户的应用，
//! 配置文件由用户自己编辑，不能要求重新构建才能改一个路径。
//!
//! 发现顺序（命中第一个即停止）：
//!
//! 1. 命令行 `--config <路径>`
//! 2. 环境变量 `APP_CONFIG`
//! 3. `./config.toml`（当前工作目录）
//! 4. `$XDG_CONFIG_HOME/yunjian/config.toml`（各平台等价目录，由 `dirs` 解析）
//!
//! 四处都没有时走首次运行路径：在第 4 个位置写一份带中文注释的默认
//! `config.toml`，并以默认值启动。
//!
//! # 密钥
//!
//! 本模块的结构体里没有任何能承载 API Key 的字段，并且顶层与每张子表都开启了
//! `deny_unknown_fields`：用户若把 `api_key` 写进 `config.toml`，会立刻收到一个
//! 指名文件路径的报错，而不是被静默忽略、留下一份明文密钥。密钥只存放在操作系统
//! 钥匙串里。

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 配置文件固定文件名。
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// 指定配置文件路径的环境变量，优先级仅次于命令行 `--config`。
pub const ENV_CONFIG: &str = "APP_CONFIG";

/// 覆盖 `corpus.path` 的环境变量。
pub const ENV_CORPUS_PATH: &str = "YUNJIAN_CORPUS_PATH";

/// 覆盖 `voice.model_dir` 的环境变量。
pub const ENV_MODEL_DIR: &str = "YUNJIAN_MODEL_DIR";

/// `ai.provider` 的哨兵值：不配置任何服务商，只使用随包预生成的赏析。
pub const PROVIDER_NONE: &str = "none";

const APP_DIR: &str = "yunjian";

/// 顶层配置。六张子表全部可省略，空文件是合法配置。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// 应用名称与通用数据目录。
    pub app: AppConfig,
    /// 语料库发现与落地配置。
    pub corpus: CorpusConfig,
    /// 日志级别、格式和文件目录配置。
    pub logger: LoggerConfig,
    /// AI 服务商、模型和生成参数配置。
    pub ai: AiConfig,
    /// 语音模型目录与运行参数配置。
    pub voice: VoiceConfig,
    /// 背诵评级与复习排程配置。
    pub recite: ReciteConfig,
}

/// `[app]`
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// 显示名称，用于窗口标题与日志字段。
    pub name: String,
    /// 没有专属配置项的用户数据（复习记录、赏析缓存）的默认落地目录。
    ///
    /// 各子系统目录彼此独立：改这里不会移动 `corpus.data_dir`、`logger.dir`
    /// 或 `voice.model_dir`。这是刻意的——配置不做任何跨字段推导。
    pub data_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "云笺".to_owned(),
            data_dir: default_data_root(),
        }
    }
}

/// `[corpus]`
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CorpusConfig {
    /// 显式指定的只读语料库文件；为 `None` 时按语料解析顺序回退。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// 首次运行把压缩语料校验、解压、原子落地到的目录。
    pub data_dir: PathBuf,
    /// 显式指定的 `.db.gz` 归档；为 `None` 时使用随包或已下载的副本。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<PathBuf>,
}

impl Default for CorpusConfig {
    fn default() -> Self {
        Self {
            path: None,
            data_dir: default_data_root().join("corpus"),
            archive: None,
        }
    }
}

/// `[logger]`
///
/// 只是数据：本模块不初始化任何日志设施，由 `logger` 模块消费这些字段。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggerConfig {
    /// 日志级别；`RUST_LOG` 优先级高于此项。
    pub level: String,
    /// 是否额外输出 JSON 结构化日志。
    pub json: bool,
    /// 按天滚动的日志文件目录。控制台日志固定走 stderr。
    pub dir: PathBuf,
    /// 日志文件名前缀，实际文件形如 `yunjian.2026-08-10`。
    pub file_prefix: String,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            json: false,
            dir: default_data_root().join("logs"),
            file_prefix: APP_DIR.to_owned(),
        }
    }
}

/// `[ai]`
///
/// 这里**没有**、并且永远不会有承载 API Key 的字段；密钥只存放在操作系统钥匙串。
/// `Serialize` 的存在正是为了让守卫测试能列出全部字段名并否决新增的密钥字段。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    /// 服务商标识；[`PROVIDER_NONE`] 表示不配置服务商。
    pub provider: String,
    /// 模型名；`None` 表示交给服务商默认模型。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 自定义 base URL（自建网关或代理）。禁止在 URL 里携带密钥。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// 采样温度，`0.0` 表示尽量确定性输出。
    pub temperature: f64,
    /// 提示词模板版本，用于让缓存随模板演进而失效。
    pub prompt_template_version: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: PROVIDER_NONE.to_owned(),
            model: None,
            endpoint: None,
            temperature: 0.0,
            prompt_template_version: "v1".to_owned(),
        }
    }
}

/// `[voice]`
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceConfig {
    /// 语音模型缓存目录。模型永不随安装包分发。
    pub model_dir: PathBuf,
    /// TTS 模型名，必须是模型清单中许可为 MIT 或 Apache-2.0 的条目。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts_model: Option<String>,
    /// ASR 模型名，许可要求同上。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr_model: Option<String>,
    /// 是否允许按需下载模型；关闭后缺模型直接降级为键入练习。
    pub allow_download: bool,
    /// 朗读节奏。
    pub prosody: VoiceProsodyConfig,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model_dir: default_data_root().join("models"),
            tts_model: None,
            asr_model: None,
            allow_download: true,
            prosody: VoiceProsodyConfig::default(),
        }
    }
}

/// `[voice.prosody]`
///
/// 朗读节奏的两个停顿时长。**它们是配置项而不是常量，理由是可测性而非灵活性**：合成引擎
/// 既无 SSML、其 `silence_scale` 也已报损，所以节奏只能由逐音步合成加 Rust 侧插静音得到，
/// 而验收断言的是「间隔不短于配置值」。做成配置之后，把 120 调成 150 不会让任何测试失效——
/// 测试的结构不随调参而改。写成常量则每次调参都要同步改一个硬编码数字，那正是会被改漏的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceProsodyConfig {
    /// 同一行内相邻音步之间的静音，毫秒。
    pub foot_pause_ms: u32,
    /// 行与行之间的静音，毫秒。
    pub line_pause_ms: u32,
}

impl Default for VoiceProsodyConfig {
    fn default() -> Self {
        Self {
            foot_pause_ms: 120,
            line_pause_ms: 400,
        }
    }
}

/// `[recite]`
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReciteConfig {
    /// 打字分数到 FSRS 等级的阈值。
    pub grading: GradingConfig,
}

/// `[recite.grading]`
///
/// 四个阈值按 [`GradingConfig`] 的字段名落盘；评级规则本身在背诵 crate 中按严格优先级
/// 求值。拒绝信号来自评分内核，不是评级阈值。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GradingConfig {
    /// 完整度低于该值时评为 Again。
    pub again_completeness_below: f32,
    /// 宽松准确率低于该值时评为 Hard。
    pub hard_accuracy_lenient_below: f32,
    /// 回读次数高于该值时评为 Hard，并禁止评为 Easy。
    pub hard_rerecitation_above: usize,
    /// 首次作答的严格准确率达到该值时才可评为 Easy。
    pub easy_accuracy_strict_at_least: f32,
}

impl Default for GradingConfig {
    fn default() -> Self {
        Self {
            again_completeness_below: 0.6,
            hard_accuracy_lenient_below: 0.85,
            hard_rerecitation_above: 0,
            easy_accuracy_strict_at_least: 0.97,
        }
    }
}

/// 用户数据根目录：优先平台数据目录，其次 `~/.yunjian`，最后临时目录。
///
/// 绝不回退到可执行文件旁边——安装目录通常不可写。
fn default_data_root() -> PathBuf {
    if let Some(dir) = dirs::data_dir() {
        return dir.join(APP_DIR);
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".yunjian");
    }
    std::env::temp_dir().join(APP_DIR)
}

/// 用户配置目录下的 `config.toml` 路径，即发现顺序的第 4 级。
pub fn user_config_path(app: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(app).join(CONFIG_FILE_NAME))
}

/// 按发现顺序挑出该读哪个文件；`None` 表示四处都没有，交给首次运行路径。
///
/// 抽成纯函数是为了能在不改动进程环境和工作目录的前提下验证优先级。
/// `cli_path` 与 `env_path` 不做存在性检查：显式指向不存在的文件必须报错，
/// 而不是静默降级到下一级。
fn resolve_path(
    cli_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    cwd_candidate: &Path,
    user_candidate: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = cli_path {
        return Some(path);
    }
    if let Some(path) = env_path {
        return Some(path);
    }
    if cwd_candidate.exists() {
        return Some(cwd_candidate.to_path_buf());
    }
    user_candidate
        .filter(|path| path.exists())
        .map(Path::to_path_buf)
}

impl Config {
    /// 读取并解析指定路径的 TOML。错误信息一律指名文件路径。
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("解析 TOML 失败: {}", path.display()))
    }

    /// 按发现顺序定位并解析配置，随后应用两个环境变量覆盖。
    pub fn discover(cli_path: Option<PathBuf>, app: &str) -> Result<Self> {
        let env_path = std::env::var_os(ENV_CONFIG)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let cwd_candidate = PathBuf::from(CONFIG_FILE_NAME);
        let user_candidate = user_config_path(app);

        let mut config = match resolve_path(
            cli_path,
            env_path,
            &cwd_candidate,
            user_candidate.as_deref(),
        ) {
            Some(path) => Self::from_path(path)?,
            None => Self::materialize_default(user_candidate.as_deref()),
        };
        config.apply_env_overrides();
        Ok(config)
    }

    /// 首次运行：写一份带注释的默认配置，并以默认值启动。
    fn materialize_default(user_path: Option<&Path>) -> Self {
        let config = Self::default();
        if let Some(path) = user_path {
            // 写模板是顺带行为。只读 home、受限容器等场景下写不进去也必须能启动，
            // 所以这里刻意吞掉写入错误而不是上抛。
            let _ = write_template(path, &config);
        }
        config
    }

    /// 应用两个——且仅有两个——环境变量覆盖。
    ///
    /// 刻意不做通用合并层：想覆盖别的字段就改 `config.toml`。
    fn apply_env_overrides(&mut self) {
        self.apply_overrides(env_path(ENV_CORPUS_PATH), env_path(ENV_MODEL_DIR));
    }

    fn apply_overrides(&mut self, corpus_path: Option<PathBuf>, model_dir: Option<PathBuf>) {
        if let Some(path) = corpus_path {
            self.corpus.path = Some(path);
        }
        if let Some(dir) = model_dir {
            self.voice.model_dir = dir;
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn write_template(path: &Path, config: &Config) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, default_config_toml(config))?;
    std::fs::rename(&tmp, path)
}

/// 把值渲染成合法的 TOML 字符串字面量，交给 `toml` 自己转义。
///
/// Windows 路径里的反斜杠必须转义，手写引号会写出解析不了的文件。
fn quote(value: &str) -> String {
    toml::Value::from(value).to_string()
}

fn quote_path(path: &Path) -> String {
    quote(&path.to_string_lossy())
}

/// 生成带中文注释的 `config.toml` 文本。解析它必须得到与入参等价的配置。
pub fn default_config_toml(config: &Config) -> String {
    format!(
        r#"# 云笺 config.toml
#
# 首次运行自动生成，可直接手工编辑；删除后下次启动会重新生成。
# 所有小节与键都是可选的：整段删掉即回退到注释里标注的默认值。
#
# 发现顺序（命中第一个即停止）：
#   1. --config <路径>
#   2. 环境变量 {env_config}
#   3. ./{file_name}（当前工作目录）
#   4. 本文件
#
# 仅有两个环境变量会覆盖本文件里的值：
#   {env_corpus}  覆盖 corpus.path
#   {env_model}    覆盖 voice.model_dir
#
# 【安全】不要把 API Key 写进本文件。密钥存放在操作系统钥匙串里，本文件既没有
# 任何用于承载密钥的字段，也不接受未知键——写错或多写会直接报错。

[app]
# 显示名称，用于窗口标题与日志字段。
name = {app_name}
# 没有专属配置项的用户数据（复习记录、赏析缓存）默认落在这里。
# 注意：各子系统目录相互独立，改这里不会移动下面三个目录。
data_dir = {app_data_dir}

[corpus]
# 显式指定只读语料库文件；留空则按内置顺序解析。
# path = "/path/to/yunjian-corpus.db"
# 首次运行把压缩语料校验解压后落地到的目录。
data_dir = {corpus_data_dir}
# 显式指定 .db.gz 归档；留空则使用随包或已下载的副本。
# archive = "/path/to/yunjian-corpus.db.gz"

[logger]
# 日志级别：trace / debug / info / warn / error。RUST_LOG 优先级高于此项。
level = {logger_level}
# 是否额外输出 JSON 结构化日志。
json = {logger_json}
# 按天滚动的日志文件目录。控制台日志固定走 stderr，永不写 stdout。
dir = {logger_dir}
# 日志文件名前缀，实际文件形如 {logger_prefix_sample}.2026-08-10。
file_prefix = {logger_prefix}

[ai]
# AI 赏析服务商；"{provider_none}" 表示不配置服务商，只用随包预生成的赏析。
# 可选：{provider_none} / deepseek / kimi / moonshot / qwen / zai / openrouter /
#       ollama / openai / anthropic
provider = {ai_provider}
# 模型名；留空表示交给服务商默认模型。
# model = "deepseek-chat"
# 自定义 base URL（自建网关或代理）。禁止在 URL 里携带密钥。
# endpoint = "https://example.com/v1"
# 采样温度，0.0 表示尽量确定性输出。
temperature = {ai_temperature:?}
# 提示词模板版本，用于让缓存随模板演进而失效。
prompt_template_version = {ai_template_version}

[voice]
# 语音模型缓存目录。模型永不随安装包分发，按需下载并校验后使用。
model_dir = {voice_model_dir}
# TTS / ASR 模型名，必须是模型清单中许可为 MIT 或 Apache-2.0 的条目。
# tts_model = "vits-melo-tts-zh_en"
# asr_model = "sherpa-onnx-streaming-zipformer-zh"
# 是否允许按需下载模型；关闭后缺模型将直接降级为键入练习。
allow_download = {voice_allow_download}

[recite.grading]
# 按以下严格优先级评级：拒绝或完整度不足 -> Again；宽松准确率不足或存在回读 -> Hard；
# 首次作答且严格准确率达标、无回读 -> Easy；其余 -> Good。
again_completeness_below = {again_completeness_below:?}
hard_accuracy_lenient_below = {hard_accuracy_lenient_below:?}
hard_rerecitation_above = {hard_rerecitation_above}
easy_accuracy_strict_at_least = {easy_accuracy_strict_at_least:?}

[voice.prosody]
# 朗读时音步之间与行之间插入的静音，毫秒。引擎既无 SSML、其静音参数也已报损，节奏由
# 逐音步合成加 Rust 侧插静音得到，这两个值就是那两段静音的时长。
foot_pause_ms = {voice_foot_pause_ms}
line_pause_ms = {voice_line_pause_ms}
"#,
        env_config = ENV_CONFIG,
        env_corpus = ENV_CORPUS_PATH,
        env_model = ENV_MODEL_DIR,
        file_name = CONFIG_FILE_NAME,
        provider_none = PROVIDER_NONE,
        app_name = quote(&config.app.name),
        app_data_dir = quote_path(&config.app.data_dir),
        corpus_data_dir = quote_path(&config.corpus.data_dir),
        logger_level = quote(&config.logger.level),
        logger_json = config.logger.json,
        logger_dir = quote_path(&config.logger.dir),
        logger_prefix = quote(&config.logger.file_prefix),
        logger_prefix_sample = config.logger.file_prefix,
        ai_provider = quote(&config.ai.provider),
        ai_temperature = config.ai.temperature,
        ai_template_version = quote(&config.ai.prompt_template_version),
        voice_model_dir = quote_path(&config.voice.model_dir),
        voice_allow_download = config.voice.allow_download,
        again_completeness_below = config.recite.grading.again_completeness_below,
        hard_accuracy_lenient_below = config.recite.grading.hard_accuracy_lenient_below,
        hard_rerecitation_above = config.recite.grading.hard_rerecitation_above,
        easy_accuracy_strict_at_least = config.recite.grading.easy_accuracy_strict_at_least,
        voice_foot_pause_ms = config.voice.prosody.foot_pause_ms,
        voice_line_pause_ms = config.voice.prosody.line_pause_ms,
    )
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// 在 `main` 里尽早调用一次，之后各处用 [`get_config`] 借用。
///
/// 重复调用返回 `Err` 而不是静默忽略：初始化顺序错了应当被看见。
pub fn init_config(cli_path: Option<PathBuf>, app: &str) -> Result<&'static Config> {
    let config = Config::discover(cli_path, app)?;
    CONFIG
        .set(config)
        .map_err(|_| anyhow!("配置已初始化，init_config() 只能调用一次"))?;
    Ok(CONFIG.get().expect("刚刚写入"))
}

/// 借用全局配置。
///
/// # Panics
///
/// [`init_config`] 之前调用会 panic：初始化顺序是程序员错误，不是运行时状况。
pub fn get_config() -> &'static Config {
    CONFIG
        .get()
        .expect("config not initialized; call init_config() first")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 不引入 `tempfile` 依赖的最小临时目录，析构时递归删除。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("系统时间晚于 UNIX 纪元")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "yunjian-config-{tag}-{}-{nanos}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const FULL_TOML: &str = r#"
[app]
name = "云笺-测试"
data_dir = "/tmp/yj/app"

[corpus]
path = "/tmp/yj/corpus.db"
data_dir = "/tmp/yj/corpus"
archive = "/tmp/yj/corpus.db.gz"

[logger]
level = "debug"
json = true
dir = "/tmp/yj/logs"
file_prefix = "yj"

[ai]
provider = "deepseek"
model = "deepseek-chat"
endpoint = "https://example.invalid/v1"
temperature = 0.7
prompt_template_version = "v2"

[voice]
model_dir = "/tmp/yj/models"
tts_model = "vits-melo-tts-zh_en"
asr_model = "sherpa-onnx-streaming-zipformer-zh"
allow_download = false

[recite.grading]
again_completeness_below = 0.7
hard_accuracy_lenient_below = 0.9
hard_rerecitation_above = 1
easy_accuracy_strict_at_least = 0.99

[voice.prosody]
foot_pause_ms = 150
line_pause_ms = 500
"#;

    /// 每个字段都填非默认值、且**穷尽**书写字面量（不用 `..Default::default()`）：
    /// 往结构体里加字段会让本函数编译失败，逼着作者同步默认模板与守卫测试。
    fn probe_config() -> Config {
        Config {
            app: AppConfig {
                name: "probe".to_owned(),
                data_dir: PathBuf::from("/probe/app"),
            },
            corpus: CorpusConfig {
                path: Some(PathBuf::from("/probe/corpus.db")),
                data_dir: PathBuf::from("/probe/corpus"),
                archive: Some(PathBuf::from("/probe/corpus.db.gz")),
            },
            logger: LoggerConfig {
                level: "trace".to_owned(),
                json: true,
                dir: PathBuf::from("/probe/logs"),
                file_prefix: "probe".to_owned(),
            },
            ai: AiConfig {
                provider: "deepseek".to_owned(),
                model: Some("deepseek-chat".to_owned()),
                endpoint: Some("https://example.invalid/v1".to_owned()),
                temperature: 0.7,
                prompt_template_version: "v2".to_owned(),
            },
            voice: VoiceConfig {
                model_dir: PathBuf::from("/probe/models"),
                tts_model: Some("tts".to_owned()),
                asr_model: Some("asr".to_owned()),
                allow_download: false,
                prosody: VoiceProsodyConfig {
                    foot_pause_ms: 90,
                    line_pause_ms: 333,
                },
            },
            recite: ReciteConfig {
                grading: GradingConfig {
                    again_completeness_below: 0.7,
                    hard_accuracy_lenient_below: 0.9,
                    hard_rerecitation_above: 1,
                    easy_accuracy_strict_at_least: 0.99,
                },
            },
        }
    }

    fn write_named(dir: &TempDir, name: &str, app_name: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建父目录");
        }
        fs::write(&path, format!("[app]\nname = {}\n", quote(app_name))).expect("写入配置");
        path
    }

    #[test]
    fn full_toml_parses_every_field() {
        let config: Config = toml::from_str(FULL_TOML).expect("完整示例应当解析成功");

        assert_eq!(config.app.name, "云笺-测试");
        assert_eq!(config.app.data_dir, PathBuf::from("/tmp/yj/app"));
        assert_eq!(config.corpus.path, Some(PathBuf::from("/tmp/yj/corpus.db")));
        assert_eq!(config.corpus.data_dir, PathBuf::from("/tmp/yj/corpus"));
        assert_eq!(
            config.corpus.archive,
            Some(PathBuf::from("/tmp/yj/corpus.db.gz"))
        );
        assert_eq!(config.logger.level, "debug");
        assert!(config.logger.json);
        assert_eq!(config.logger.dir, PathBuf::from("/tmp/yj/logs"));
        assert_eq!(config.logger.file_prefix, "yj");
        assert_eq!(config.ai.provider, "deepseek");
        assert_eq!(config.ai.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(
            config.ai.endpoint.as_deref(),
            Some("https://example.invalid/v1")
        );
        assert!((config.ai.temperature - 0.7).abs() < f64::EPSILON);
        assert_eq!(config.ai.prompt_template_version, "v2");
        assert_eq!(config.voice.model_dir, PathBuf::from("/tmp/yj/models"));
        assert_eq!(
            config.voice.tts_model.as_deref(),
            Some("vits-melo-tts-zh_en")
        );
        assert_eq!(
            config.voice.asr_model.as_deref(),
            Some("sherpa-onnx-streaming-zipformer-zh")
        );
        assert!(!config.voice.allow_download);
        assert_eq!(config.recite.grading.again_completeness_below, 0.7);
        assert_eq!(config.recite.grading.hard_accuracy_lenient_below, 0.9);
        assert_eq!(config.recite.grading.hard_rerecitation_above, 1);
        assert_eq!(config.recite.grading.easy_accuracy_strict_at_least, 0.99);
        assert_eq!(config.voice.prosody.foot_pause_ms, 150);
        assert_eq!(config.voice.prosody.line_pause_ms, 500);
    }

    #[test]
    fn toml_omitting_every_optional_table_still_parses() {
        assert_eq!(
            toml::from_str::<Config>("").expect("空文件应当合法"),
            Config::default()
        );

        let partial: Config =
            toml::from_str("[app]\nname = \"仅此一项\"\n").expect("只写一张表也应当合法");
        assert_eq!(partial.app.name, "仅此一项");
        assert_eq!(partial.app.data_dir, AppConfig::default().data_dir);
        assert_eq!(partial.corpus, CorpusConfig::default());
        assert_eq!(partial.logger, LoggerConfig::default());
        assert_eq!(partial.ai, AiConfig::default());
        assert_eq!(partial.voice, VoiceConfig::default());
        assert_eq!(partial.recite, ReciteConfig::default());
    }

    #[test]
    fn defaults_match_the_documented_contract() {
        let config = Config::default();
        assert_eq!(config.logger.level, "info");
        assert!(!config.logger.json);
        assert_eq!(config.logger.file_prefix, "yunjian");
        assert_eq!(config.ai.provider, PROVIDER_NONE);
        assert!((config.ai.temperature - 0.0).abs() < f64::EPSILON);
        assert!(config.voice.allow_download);
        assert_eq!(config.corpus.path, None);
        assert_eq!(config.recite.grading, GradingConfig::default());
    }

    #[test]
    fn discovery_prefers_earlier_sources() {
        let dir = TempDir::new("precedence");
        let cli = write_named(&dir, "cli.toml", "cli");
        let env = write_named(&dir, "env.toml", "env");
        let cwd = write_named(&dir, "cwd/config.toml", "cwd");
        let user = write_named(&dir, "user/yunjian/config.toml", "user");

        let chosen = resolve_path(Some(cli.clone()), Some(env.clone()), &cwd, Some(&user))
            .expect("第 1 级应当命中");
        assert_eq!(chosen, cli);
        assert_eq!(Config::from_path(&chosen).expect("解析").app.name, "cli");

        let chosen =
            resolve_path(None, Some(env.clone()), &cwd, Some(&user)).expect("第 2 级应当命中");
        assert_eq!(chosen, env);
        assert_eq!(Config::from_path(&chosen).expect("解析").app.name, "env");

        let chosen = resolve_path(None, None, &cwd, Some(&user)).expect("第 3 级应当命中");
        assert_eq!(chosen, cwd);
        assert_eq!(Config::from_path(&chosen).expect("解析").app.name, "cwd");

        let missing_cwd = dir.join("no-such/config.toml");
        let chosen = resolve_path(None, None, &missing_cwd, Some(&user)).expect("第 4 级应当命中");
        assert_eq!(chosen, user);
        assert_eq!(Config::from_path(&chosen).expect("解析").app.name, "user");

        let missing_user = dir.join("no-such/user.toml");
        assert_eq!(
            resolve_path(None, None, &missing_cwd, Some(&missing_user)),
            None
        );
        assert_eq!(resolve_path(None, None, &missing_cwd, None), None);
    }

    #[test]
    fn explicit_sources_do_not_silently_fall_through() {
        let dir = TempDir::new("explicit-missing");
        let missing = dir.join("absent.toml");
        let present = write_named(&dir, "config.toml", "cwd");

        assert_eq!(
            resolve_path(Some(missing.clone()), None, &present, None),
            Some(missing.clone())
        );
        assert_eq!(
            resolve_path(None, Some(missing.clone()), &present, None),
            Some(missing.clone())
        );

        let err = Config::from_path(&missing).expect_err("读取不存在的文件应当失败");
        assert!(
            format!("{err}").contains(&missing.display().to_string()),
            "错误信息必须指名路径: {err}"
        );
    }

    /// 唯一一条允许承载密钥的路径是操作系统钥匙串。这里用序列化后的字段名做守卫，
    /// 将来有人往配置里加 `api_key` / `token` 之类字段，本测试即刻变红。
    #[test]
    fn config_carries_no_credential_shaped_field() {
        const FORBIDDEN: [&str; 4] = ["key", "secret", "token", "password"];

        let assert_clean = |label: &str, value: &serde_json::Value| {
            let mut stack = vec![value.clone()];
            while let Some(node) = stack.pop() {
                if let Some(object) = node.as_object() {
                    for (name, child) in object {
                        let lowered = name.to_ascii_lowercase();
                        for word in FORBIDDEN {
                            assert!(
                                !lowered.contains(word),
                                "{label} 出现疑似凭据字段 `{name}`（命中 `{word}`）"
                            );
                        }
                        stack.push(child.clone());
                    }
                }
            }
        };

        let default_ai = serde_json::to_value(AiConfig::default()).expect("AiConfig 可序列化");
        assert_clean("AiConfig::default()", &default_ai);

        let filled = probe_config();
        let filled_ai = serde_json::to_value(&filled.ai).expect("AiConfig 可序列化");
        assert_clean("填满的 AiConfig", &filled_ai);
        assert_eq!(
            filled_ai.as_object().expect("对象").len(),
            5,
            "AiConfig 只应有 provider / model / endpoint / temperature / prompt_template_version"
        );

        assert_clean(
            "填满的 Config",
            &serde_json::to_value(&filled).expect("Config 可序列化"),
        );
    }

    #[test]
    fn credential_key_in_config_file_is_rejected_by_name() {
        let err = toml::from_str::<Config>("[ai]\napi_key = \"sk-TESTKEY123\"\n")
            .expect_err("未知键必须报错，绝不能静默忽略一份明文密钥");
        assert!(
            format!("{err}").contains("api_key"),
            "报错应当指出是哪个键: {err}"
        );
    }

    #[test]
    fn invalid_toml_reports_the_offending_path() {
        let dir = TempDir::new("invalid");
        let path = dir.join("broken.toml");
        fs::write(&path, "[app\nname = ").expect("写入坏文件");

        let err = Config::from_path(&path).expect_err("语法错误必须返回 Err");
        let rendered = format!("{err}");
        assert!(
            rendered.contains(&path.display().to_string()),
            "错误信息必须指名路径: {err}"
        );
        assert!(
            rendered.contains("解析 TOML 失败"),
            "错误信息应当说明失败阶段: {err}"
        );
    }

    /// 在子进程里跑一条被 `#[ignore]` 标记的用例。
    ///
    /// 需要子进程是因为 `init_config` 写全局 `OnceLock`、而改环境变量在 Rust 2024
    /// 里是 `unsafe` 且会污染并行执行的其他用例。
    fn run_child(test_name: &str, configure: impl FnOnce(&mut Command)) {
        let exe = std::env::current_exe().expect("取测试可执行文件路径");
        let mut command = Command::new(exe);
        command
            .arg(format!("config::tests::{test_name}"))
            .args(["--exact", "--ignored"])
            .env_remove(ENV_CONFIG)
            .env_remove(ENV_CORPUS_PATH)
            .env_remove(ENV_MODEL_DIR);
        configure(&mut command);
        let output = command.output().expect("拉起子进程");

        // 必须同时打出 stdout：libtest 把失败用例的 panic 信息收进自己的缓冲区，
        // 在 `failures:` 段落里从 **stdout** 输出。只打 stderr 会得到一条空诊断，
        // 于是「子进程失败了」这个事实有了，「为什么失败」却完全看不到。
        assert!(
            output.status.success(),
            "子进程 {test_name} 断言失败:\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // 过滤不到任何用例时 libtest 同样返回 0，所以必须确认子进程真的跑了那一条，
        // 否则改个测试名就会让本测试变成永远通过的空壳。
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "子进程没有真正执行到 {test_name}:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    /// 把子进程各平台的家目录/配置目录都指到隔离目录，让 `dirs` 的解析可预测。
    ///
    /// Windows 上光设环境变量不够。`dirs::config_dir()` 走的是
    /// `SHGetKnownFolderPath(FOLDERID_RoamingAppData)`，它从 HKCU 的 User Shell Folders
    /// 取到 REG_EXPAND_SZ 值 `%USERPROFILE%\AppData\Roaming` 并用**进程环境**展开，
    /// 而且调用方没有传 `KF_FLAG_DONT_VERIFY`，**目标目录不存在时它直接返回错误**，
    /// 于是 `config_dir()` 变成 `None`。所以必须先把这两层目录建出来：
    /// 建了，隔离才真的落在临时目录里；不建，隔离测试在 Windows 上以
    /// 「隔离环境应当有配置目录」失败（已在 windows-latest 上实测）。
    fn redirect_home(command: &mut Command, home: &Path) {
        for sub in ["AppData/Roaming", "AppData/Local"] {
            fs::create_dir_all(home.join(sub)).expect("预建 Windows 的 AppData 子目录");
        }
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
        ] {
            command.env(key, home);
        }
    }

    /// QA 场景的原样复现：把 `APP_CONFIG` 指向语法错误的 TOML，`init_config`
    /// 必须返回带路径的 `Err` 而不是 panic。
    #[test]
    fn init_config_via_env_returns_err_for_invalid_toml() {
        let dir = TempDir::new("env-invalid");
        let path = dir.join("broken.toml");
        fs::write(&path, "[app\nname = ").expect("写入坏文件");

        run_child("child_asserts_env_invalid_toml", |command| {
            command
                .env(ENV_CONFIG, &path)
                .env("YUNJIAN_TEST_BAD_CONFIG", &path);
        });
    }

    /// 四级发现全部落空时的完整首次运行链路：`discover` 写出默认文件 ->
    /// `init_config` 成功 -> `get_config` 借到同一份配置。
    #[test]
    fn first_run_end_to_end_in_an_empty_environment() {
        let dir = TempDir::new("e2e-first-run");
        let home = dir.join("home");
        let cwd = dir.join("cwd");
        fs::create_dir_all(&home).expect("创建隔离家目录");
        fs::create_dir_all(&cwd).expect("创建空工作目录");

        run_child("child_asserts_first_run", |command| {
            redirect_home(command, &home);
            command.current_dir(&cwd);
        });
    }

    #[test]
    #[ignore = "由 first_run_end_to_end_in_an_empty_environment 带着隔离的家目录拉起"]
    fn child_asserts_first_run() {
        let expected_path = user_config_path(APP_DIR).expect("隔离环境应当有配置目录");
        assert!(!expected_path.exists(), "前置条件：配置文件此时还不该存在");
        assert!(
            !Path::new(CONFIG_FILE_NAME).exists(),
            "前置条件：工作目录应当是空的"
        );

        let config = init_config(None, APP_DIR).expect("四级全空必须走首次运行而非报错");

        assert_eq!(config, &Config::default());
        assert!(
            expected_path.exists(),
            "首次运行应当在用户配置目录写出默认文件"
        );
        let written = fs::read_to_string(&expected_path).expect("读回默认配置");
        assert!(
            written.starts_with("# 云笺 config.toml"),
            "写出的文件应当带注释头"
        );
        assert!(
            std::ptr::eq(get_config(), config),
            "get_config 必须返回同一实例"
        );
        assert!(
            init_config(None, APP_DIR).is_err(),
            "重复初始化必须报错，而不是静默忽略第二次调用"
        );
    }

    #[test]
    #[ignore = "由 init_config_via_env_returns_err_for_invalid_toml 带着 APP_CONFIG 拉起"]
    fn child_asserts_env_invalid_toml() {
        let expected = std::env::var("YUNJIAN_TEST_BAD_CONFIG").expect("父进程应当传入路径");
        let err = init_config(None, APP_DIR).expect_err("语法错误必须返回 Err 而非 panic");
        let rendered = format!("{err}");
        assert!(rendered.contains(&expected), "错误信息必须指名路径: {err}");
    }

    #[test]
    #[should_panic(expected = "config not initialized; call init_config() first")]
    fn get_config_panics_before_init() {
        let _ = get_config();
    }

    #[test]
    fn default_template_round_trips_to_defaults() {
        let expected = Config::default();
        let rendered = default_config_toml(&expected);
        let parsed: Config = toml::from_str(&rendered).expect("默认模板必须是合法 TOML");
        assert_eq!(
            parsed, expected,
            "模板解析结果必须与 Config::default() 一致"
        );
    }

    #[test]
    fn default_template_mentions_every_field() {
        let defaults = Config::default();
        let rendered = default_config_toml(&defaults);
        let parsed = toml::from_str::<toml::Value>(&rendered).expect("模板必须是合法 TOML");
        let serialized_defaults =
            toml::Value::try_from(&defaults).expect("默认配置可序列化为 TOML");
        let probe = toml::Value::try_from(probe_config()).expect("探针配置可序列化为 TOML");

        for (section, table) in probe.as_table().expect("顶层是表") {
            for key in table.as_table().expect("子表是表").keys() {
                let has_default_value = serialized_defaults
                    .get(section)
                    .and_then(|value| value.get(key))
                    .is_some();
                if has_default_value {
                    assert!(
                        parsed
                            .get(section)
                            .and_then(|value| value.get(key))
                            .is_some(),
                        "默认模板缺少 {section}.{key}"
                    );
                } else {
                    assert!(
                        rendered.contains(&format!("# {key} = ")),
                        "默认模板未以注释形式提到可选项 {section}.{key}"
                    );
                }
            }
        }
    }

    #[test]
    fn first_run_materializes_a_commented_default_file() {
        let dir = TempDir::new("first-run");
        let path = dir.join("nested/yunjian/config.toml");

        let config = Config::materialize_default(Some(&path));

        assert_eq!(config, Config::default());
        assert!(path.exists(), "首次运行应当写出默认配置");
        let written = fs::read_to_string(&path).expect("读回默认配置");
        assert!(
            written.starts_with("# 云笺 config.toml"),
            "写出的文件应当带注释头"
        );
        assert!(
            written.contains("不要把 API Key 写进本文件"),
            "写出的文件应当带密钥警示"
        );
        assert_eq!(
            toml::from_str::<Config>(&written).expect("写出的文件必须能解析"),
            Config::default()
        );
        assert!(
            !dir.join("nested/yunjian/config.toml.tmp").exists(),
            "不应残留临时文件"
        );
    }

    #[test]
    fn first_run_never_overwrites_an_existing_file() {
        let dir = TempDir::new("no-clobber");
        let path = write_named(&dir, "config.toml", "用户自己写的");

        let _ = Config::materialize_default(Some(&path));

        assert_eq!(
            fs::read_to_string(&path).expect("读回"),
            "[app]\nname = \"用户自己写的\"\n"
        );
    }

    #[test]
    fn only_two_env_overrides_exist() {
        let mut config = Config::default();
        let baseline = config.clone();

        config.apply_overrides(
            Some(PathBuf::from("/override/corpus.db")),
            Some(PathBuf::from("/override/models")),
        );

        assert_eq!(
            config.corpus.path,
            Some(PathBuf::from("/override/corpus.db"))
        );
        assert_eq!(config.voice.model_dir, PathBuf::from("/override/models"));
        assert_eq!(config.app, baseline.app);
        assert_eq!(config.logger, baseline.logger);
        assert_eq!(config.ai, baseline.ai);
        assert_eq!(config.recite, baseline.recite);
        assert_eq!(config.corpus.data_dir, baseline.corpus.data_dir);
        assert_eq!(config.corpus.archive, baseline.corpus.archive);

        let mut untouched = Config::default();
        untouched.apply_overrides(None, None);
        assert_eq!(untouched, baseline, "环境变量缺席时不得改动任何字段");

        assert_eq!(ENV_CORPUS_PATH, "YUNJIAN_CORPUS_PATH");
        assert_eq!(ENV_MODEL_DIR, "YUNJIAN_MODEL_DIR");
        assert_eq!(ENV_CONFIG, "APP_CONFIG");
    }

    #[test]
    fn user_config_path_ends_with_app_scoped_config_file() {
        let path = user_config_path(APP_DIR).expect("测试环境应当有配置目录");
        assert!(
            path.ends_with(PathBuf::from(APP_DIR).join(CONFIG_FILE_NAME)),
            "{}",
            path.display()
        );
    }
}
