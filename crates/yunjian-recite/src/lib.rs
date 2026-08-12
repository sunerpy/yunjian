//! 云笺背诵 crate。
//!
//! 承载对齐评分内核与 FSRS 复习调度。评分内核先在键入输入上验证，
//! 再接入语音识别，以便区分对齐缺陷与听错。

#![warn(missing_docs)]

pub mod align;
pub mod modes;
pub mod score;

pub use align::{AlignOp, Alignment, align};
pub use modes::{ClozeOptions, MASK_CHARACTER, MaskStage, PracticeMode, PracticeSession};
pub use score::{
    BiasedHyp, OpsSummary, Poem, RelativeRhythm, TypedAttempt, TypedScore, VoicePracticeFeedback,
    score_typed,
};
