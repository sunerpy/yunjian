//! 云笺背诵 crate。
//!
//! 承载对齐评分内核与 FSRS 复习调度。评分内核先在键入输入上验证，
//! 再接入语音识别，以便区分对齐缺陷与听错。

#![warn(missing_docs)]

pub mod align;
pub mod learning;
pub mod modes;
pub mod phonetic;
pub mod queue;
pub mod schedule;
pub mod score;

pub use align::{AlignOp, Alignment, align};
pub use learning::{
    CompleteRecitation, FootPracticeRef, LearningChunk, LearningObjects, MasterySummary,
    SEGMENTATION_VERSION, WholePoem, build_learning_objects, summarize_mastery,
};
pub use modes::{ClozeOptions, MASK_CHARACTER, MaskStage, PracticeMode, PracticeSession};
pub use phonetic::{
    NEAR_HOMOPHONE_ERROR_WEIGHT, NEAR_HOMOPHONE_MAX_DISTANCE, PhoneticReview, SubstitutionClass,
    SubstitutionReview, classify_substitution, nearest_reading_distance, review_typed,
    review_typed_text,
};
pub use queue::{
    BacklogReport, BudgetConfig, DailyPressure, DailyQueueInput, DailyQueueReport, EstimatedTask,
    ObservedRetention, QueueKind, RetentionObservation, estimate_minutes, plan_daily_queue,
};
pub use schedule::{
    FSRS6_PARAMETERS, FsrsGrade, PracticeOutcome, RelearningTicket, ReviewState, ReviewSubmission,
    ReviewTicket, Scheduler, grade_typed,
};
pub use score::{
    BiasedHyp, OpsSummary, Poem, RelativeRhythm, TypedAttempt, TypedScore, VoicePracticeFeedback,
    score_typed,
};
