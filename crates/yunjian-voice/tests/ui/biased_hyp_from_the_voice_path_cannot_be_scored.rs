use yunjian_recite::{Poem, score_typed};
use yunjian_voice::recognize::{Hotwords, OnlineDecodeConfig, biased_hyp};

fn misuse(reference: &Poem) {
    let config = OnlineDecodeConfig::biased(Hotwords::from_poem("床前明月光").unwrap());
    let biased = biased_hyp(&config, "床前明月光").unwrap();
    let _ = score_typed(reference, &biased);
}

fn main() {}
