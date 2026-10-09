#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;

fn main() {
    let _ = Post::REVIEWER_ID.eq(None);
}
