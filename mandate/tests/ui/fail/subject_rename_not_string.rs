#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum S {
    #[subject(rename = true)]
    Post,
}

fn main() {}
