use mandate::{Ability, Access, Resource};

// `Note` and `Author` are only relation targets: no `Subject` variant names
// them, yet conditions through them build, check and plan, and the
// projection reaches into them.
#[derive(Clone, Debug, Resource)]
struct Doc {
    id: i64,
    #[resource(relation)]
    notes: Vec<Note>,
    #[resource(relation)]
    author: Option<Author>,
}

#[derive(Clone, Debug, Resource)]
struct Note {
    body: String,
}

#[derive(Clone, Debug, Resource)]
struct Author {
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Action)]
enum Act {
    Read,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
enum Sub {
    #[subject(resource = Doc)]
    Doc,
}

fn doc(body: &str, author: Option<&str>) -> Doc {
    Doc {
        id: 1,
        notes: vec![Note { body: body.into() }],
        author: author.map(|name| Author { name: name.into() }),
    }
}

fn main() {
    let a = Ability::<Act, Sub>::builder()
        .can(Act::Read, Sub::Doc)
        .when(
            Doc::NOTES
                .some(Note::BODY.contains("ok"))
                .and(Doc::AUTHOR.then(Author::NAME.eq("Ann"))),
        )
        .build()
        .unwrap();
    let Ok(Access::Filter(plan)) = a.access::<Doc>(Act::Read) else {
        panic!("expected a filter");
    };
    for (d, expected) in [
        (doc("ok", Some("Ann")), true),
        (doc("no", Some("Ann")), false),
        (doc("ok", None), false),
        (doc("ok", Some("Bob")), false),
    ] {
        assert_eq!(a.can(Act::Read, &d), expected, "{d:?}");
        assert_eq!(plan.eval(&d), Ok(expected), "{d:?}");
    }
    let projection = a.projection::<Doc>(Act::Read).unwrap();
    let targets: Vec<_> = projection
        .relations
        .iter()
        .map(|r| (r.relation, r.target.name()))
        .collect();
    assert_eq!(
        targets,
        [(Doc::NOTES.idx(), "Note"), (Doc::AUTHOR.idx(), "Author")]
    );
}
