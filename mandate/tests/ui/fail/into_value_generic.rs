use mandate::IntoValue;

#[derive(IntoValue)]
enum Level<T> {
    Low,
    High(T),
}

fn main() {}
