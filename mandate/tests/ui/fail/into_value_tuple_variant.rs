use mandate::IntoValue;

#[derive(IntoValue)]
enum Bad {
    Ok,
    Bad(u8),
}

fn main() {}
