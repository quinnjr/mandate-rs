#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;

fn main() {
    let _ = Post::TAGS.then(Tag::ID.eq(1));
}
