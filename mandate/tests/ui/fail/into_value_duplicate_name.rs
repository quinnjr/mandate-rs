use mandate::IntoValue;

#[derive(IntoValue)]
enum Dup {
    #[value(rename = "a")]
    One,
    #[value(rename = "a")]
    Two,
}

fn main() {}
