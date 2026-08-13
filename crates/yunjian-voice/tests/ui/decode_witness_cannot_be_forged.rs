use yunjian_voice::recognize::{DecodeWitness, UnbiasedAsrHyp};

fn forge() -> UnbiasedAsrHyp {
    let witness = DecodeWitness { _seal: () };
    UnbiasedAsrHyp {
        text: "床前明月光".to_owned(),
        witness,
    }
}

fn main() {
    let _ = forge();
}
