use yunjian_core::GradingConfig;
use yunjian_recite::{FsrsGrade, VoicePracticeFeedback, grade_typed};

fn misuse(feedback: &VoicePracticeFeedback, grading: &GradingConfig) -> FsrsGrade {
    grade_typed(feedback, true, grading)
}

fn main() {}
