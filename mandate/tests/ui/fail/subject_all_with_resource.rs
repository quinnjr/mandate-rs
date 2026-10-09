#[path = "../../common/fixture.rs"]
mod fixture;

#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum S {
    #[subject(all, resource = fixture::Post)]
    A,
}

fn main() {}
