use mandate::Resource;

#[derive(Resource)]
struct Wrapper<T> {
    inner: T,
}

fn main() {}
