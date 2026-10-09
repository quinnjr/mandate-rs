use mandate::IntoValue;

#[derive(IntoValue)]
enum Op {
    #[value(rename = "$and")]
    And,
}

fn main() {}
