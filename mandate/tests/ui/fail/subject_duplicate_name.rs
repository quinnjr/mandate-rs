#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum S {
    Post,
    #[subject(rename = "Post")]
    Article,
}

fn main() {}
