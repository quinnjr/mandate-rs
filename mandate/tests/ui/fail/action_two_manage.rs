#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Action)]
enum A {
    #[action(manage)]
    One,
    #[action(manage)]
    Two,
}

fn main() {}
