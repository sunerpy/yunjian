use yunjian_recite::{TypedScore, VoicePracticeFeedback};

fn misuse(feedback: VoicePracticeFeedback) -> TypedScore {
    feedback.into()
}

fn main() {}
