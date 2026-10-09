// `EvalError`'s variants are `#[non_exhaustive]`: code outside the crate
// cannot build one, so fields can be added without a breaking change.
fn main() {
    let _ = mandate::EvalError::NotLoaded {
        path: String::new(),
    };
}
