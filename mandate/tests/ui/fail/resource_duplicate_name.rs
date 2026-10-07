use mandate::Resource;

#[derive(Resource)]
struct Doc {
    title: String,
    #[resource(rename = "title")]
    headline: String,
}

fn main() {}
