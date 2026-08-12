use yunjian_recite::{Poem, score_typed};
use yunjian_voice::recognize::{OnlineDecodeConfig, UnbiasedAsrHyp};

fn misuse(reference: &Poem) {
    let unbiased = UnbiasedAsrHyp::from_pass(&OnlineDecodeConfig::unbiased(), "床前明月光").unwrap();
    let _ = score_typed(reference, &unbiased);
}

fn main() {}
