#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;
use mandate::Resource;

#[allow(dead_code)]
#[derive(Resource)]
struct Doc {
    #[resource(relation)]
    reviewer: Option<Box<User>>,
    #[resource(relation)]
    tags: Vec<Box<Tag>>,
}

fn main() {
    let _ = Doc::REVIEWER.then(User::NAME.eq("x"));
    let _ = Doc::REVIEWER.is_null();
    let _ = Doc::TAGS.some(Tag::ID.eq(1));
}
