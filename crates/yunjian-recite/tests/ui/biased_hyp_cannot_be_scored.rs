use yunjian_recite::{BiasedHyp, Poem, score_typed};

fn misuse(reference: &Poem, biased: BiasedHyp) {
    let _ = score_typed(reference, biased);
}

fn main() {}
