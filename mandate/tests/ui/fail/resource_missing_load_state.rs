use mandate::Resource;

#[derive(Resource)]
#[resource(load_state = loaded)]
struct Doc {
    id: i64,
}

fn main() {}
