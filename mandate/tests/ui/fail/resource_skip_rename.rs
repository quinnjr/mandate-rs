use mandate::Resource;

#[derive(Resource)]
struct Doc {
    id: i64,
    #[resource(skip, rename = "html")]
    cached_html: String,
}

fn main() {}
