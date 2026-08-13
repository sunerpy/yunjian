use serde::{Deserialize, Serialize};
use yunjian_core::api::assert_stable_api_type;

#[derive(Serialize, Deserialize)]
struct BorrowedCoreResponse<'a> {
    poem_id: &'a str,
}

fn main() {
    assert_stable_api_type::<BorrowedCoreResponse<'static>>();
}
