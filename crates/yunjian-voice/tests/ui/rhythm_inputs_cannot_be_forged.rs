use yunjian_voice::session::{RhythmInputs, SessionScore};

fn misuse(score: SessionScore) {
    let _ = RhythmInputs {
        gap_variance_ms2: 0.0,
        long_pause_count: 0,
        duration_ratio: 1.0,
        _seal: (),
    };
    let _ = score;
}

fn main() {}
