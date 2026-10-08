#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Action)]
enum A {
    #[action(rename = 5)]
    Read,
}

fn main() {}
