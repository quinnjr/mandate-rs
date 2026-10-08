use mandate::{FieldDef, Kind, Schema};

static SCHEMA: Schema = Schema::new(
    "Doc",
    &[
        FieldDef::scalar("id", Kind::Int, false),
        FieldDef::opaque("id"),
    ],
);

fn main() {
    let _ = SCHEMA.name();
}
