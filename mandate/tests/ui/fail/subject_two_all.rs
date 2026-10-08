#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum S {
    #[subject(all)]
    All,
    #[subject(all)]
    Everything,
}

fn main() {}
