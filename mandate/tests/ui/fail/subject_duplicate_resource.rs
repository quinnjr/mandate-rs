#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum S {
    #[subject(resource = Post)]
    A,
    #[subject(resource = Post)]
    B,
}

fn main() {}
