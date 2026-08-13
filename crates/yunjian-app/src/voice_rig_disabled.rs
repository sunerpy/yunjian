//! 不带 `voice` 特性时的生产装置。
//!
//! **它不是「什么也不做」的空壳，而是一个诚实回答「为什么不能录」的桩。** 每一条方法都
//! 落到 [`DegradeReason::FeatureDisabled`] 对应的解释上，于是默认构建里语音入口仍然存在、
//! 仍然可点、点了给出确定的原因与下一步动作——这与命令行把中华新韵留在 `--book` 取值域里
//! 的理由是同一条：「没有这个东西」必须能被请求到并得到明确回答。

use std::path::PathBuf;

use yunjian_core::Config;
use yunjian_voice::VoiceError;
use yunjian_voice::permission::{DegradeReason, Practice, degrade};
use yunjian_voice::platform::Platform;
use yunjian_voice::prosody::Reading;
use yunjian_voice::session::TypedFallback;

use super::{Coupling, ModelFetchOut, PartialSink, VoiceRig, corpus_body};
use crate::ipc::IpcResult;

pub(crate) struct ProductionRig;

impl VoiceRig for ProductionRig {
    fn probe(&self, _config: &Config) -> Practice {
        degrade(DegradeReason::FeatureDisabled, Platform::current())
    }

    fn body(&self, config: &Config, poem_id: &str) -> IpcResult<String> {
        corpus_body(config, poem_id)
    }

    fn read(&self, _config: &Config, _body: &str) -> IpcResult<Reading> {
        Err(VoiceError::FeatureDisabled.to_string())
    }

    fn couple(&self, _config: &Config, _partials: PartialSink) -> IpcResult<Coupling> {
        Err(VoiceError::FeatureDisabled.to_string())
    }

    fn fetch_model(
        &self,
        _config: &Config,
        _name: &str,
        _stop: &dyn Fn() -> bool,
        _progress: &mut dyn FnMut(ModelFetchOut),
    ) -> Result<PathBuf, TypedFallback> {
        Err(TypedFallback::new(
            DegradeReason::FeatureDisabled,
            Platform::current(),
            0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::ProductionRig;
    use crate::voice_ipc::{VoiceRig, degrade_reason_key};
    use yunjian_core::Config;
    use yunjian_voice::permission::{DegradeReason, Practice};

    /// 默认构建里语音必须**明确不可用并说清原因**，而不是先报可用再在别处炸。
    #[test]
    fn default_build_reports_the_feature_flag_as_the_reason() {
        let Practice::Typed { reason, message } = ProductionRig.probe(&Config::default()) else {
            panic!("未编译语音能力的构建不得报告语音可用");
        };
        assert_eq!(
            degrade_reason_key(reason),
            degrade_reason_key(DegradeReason::FeatureDisabled)
        );
        assert!(
            message.contains("打字练习"),
            "解释必须给出可走的那条路：{message}"
        );
    }
}
