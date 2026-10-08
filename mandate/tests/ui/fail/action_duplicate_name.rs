#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Action)]
enum A {
    Read,
    #[action(rename = "read")]
    View,
}

fn main() {}
