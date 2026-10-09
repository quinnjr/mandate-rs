#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;

fn main() {
    // Two errors are expected, as `Vec` is both `ToMany` and `NonNull`; the
    // `Cardinality` one is the contract under test.
    let _ = Post::TAGS.is_null();
}
