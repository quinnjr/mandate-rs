# mandate-derive

Derive macros for [`mandate-rs`](https://crates.io/crates/mandate-rs), a
CASL-style authorization library for Rust: `Action` and `Subject` for your
action and subject enums, `Resource` for the structs rules are checked
against, and `IntoValue` for enums used as field values.

Do not depend on this crate directly. `mandate-rs` re-exports the derives
under its default `derive` feature:

```rust,ignore
use mandate::{Action, IntoValue, Resource, Subject};
```

See the [`mandate-rs` documentation](https://docs.rs/mandate-rs) for how to
use them.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
