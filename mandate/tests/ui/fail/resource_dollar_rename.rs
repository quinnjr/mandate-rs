use mandate::Resource;

#[derive(Resource)]
struct Doc {
    #[resource(rename = "$and")]
    title: String,
}

fn main() {}
