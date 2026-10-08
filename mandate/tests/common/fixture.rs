// Shared test fixture. Included by integration tests (`mod common;`), by the
// library's unit tests (`crate::test_fixture`), and by UI cases (`#[path]`).
// Later tests rely on the field indices noted below.

use chrono::{DateTime, Utc};
use mandate::{IntoValue, LoadState, RelationRef, RelationSlot, Resource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoValue, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Draft,
    Published,
    Archived,
}

#[derive(Clone, Debug, Resource)]
pub struct Post {
    pub id: i64,                             // 0
    pub author_id: i64,                      // 1
    pub reviewer_id: Option<i64>,            // 2
    pub title: String,                       // 3
    pub body: String,                        // 4
    pub locked: bool,                        // 5
    pub status: Status,                      // 6
    pub published_at: Option<DateTime<Utc>>, // 7
    pub score: f64,                          // 8
    #[resource(relation)]
    pub org: Org,   // 9   ToOne, NonNull
    #[resource(relation)]
    pub reviewer: Option<User>, // 10  ToOne, Nullable
    #[resource(relation)]
    pub tags: Vec<Tag>, // 11  ToMany
    #[resource(opaque)]
    pub metadata: serde_json::Value, // 12
    #[resource(skip)]
    pub cached_html: String,
}

#[derive(Clone, Debug, Resource)]
pub struct Org {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Resource)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    #[resource(relation)]
    pub posts: Vec<Post>,
}

#[derive(Clone, Debug, Resource)]
pub struct Tag {
    pub id: i64,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Resource)]
pub struct Marker {}

/// A published post: id 1, author_id 7, reviewer None, title "Hello", body "World", locked false,
/// published_at None, score 1.0, org Org{id:3,name:"Acme"}, tags [], metadata Null, cached_html "".
pub fn post() -> Post {
    Post {
        id: 1,
        author_id: 7,
        reviewer_id: None,
        title: "Hello".into(),
        body: "World".into(),
        locked: false,
        status: Status::Published,
        published_at: None,
        score: 1.0,
        org: Org {
            id: 3,
            name: "Acme".into(),
        },
        reviewer: None,
        tags: vec![],
        metadata: serde_json::Value::Null,
        cached_html: String::new(),
    }
}

/// Names of scalar fields that were NOT loaded.
#[derive(Clone, Debug, Default)]
pub struct Loaded(pub Vec<&'static str>);

impl LoadState for Loaded {
    fn scalar_loaded(&self, f: &'static str) -> bool {
        !self.0.contains(&f)
    }
}

/// A relation slot that may not have been loaded.
#[derive(Clone, Debug)]
pub enum Lazy<S> {
    NotLoaded,
    Loaded(S),
}

impl<S: RelationSlot> RelationSlot for Lazy<S> {
    type Target = S::Target;
    type Cardinality = S::Cardinality;
    type Nullability = S::Nullability;
    fn get(&self) -> RelationRef<'_> {
        match self {
            Lazy::NotLoaded => RelationRef::NotLoaded,
            Lazy::Loaded(s) => s.get(),
        }
    }
}

#[derive(Clone, Debug, Resource)]
#[resource(load_state = loaded)]
pub struct TPost {
    pub loaded: Loaded,
    pub id: i64,                  // 0
    pub author_id: i64,           // 1
    pub reviewer_id: Option<i64>, // 2
    pub title: String,            // 3
    pub status: Status,           // 4
    pub score: f64,               // 5
    #[resource(relation)]
    pub org: Lazy<TOrg>, // 6
    #[resource(relation)]
    pub reviewer: Lazy<Option<TUser>>, // 7
    #[resource(relation)]
    pub tags: Lazy<Vec<TTag>>, // 8
}

#[derive(Clone, Debug, Resource)]
#[resource(load_state = loaded)]
pub struct TOrg {
    pub loaded: Loaded,
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Resource)]
#[resource(load_state = loaded)]
pub struct TUser {
    pub loaded: Loaded,
    pub id: i64,
    pub email: String,
}

#[derive(Clone, Debug, Resource)]
#[resource(load_state = loaded)]
pub struct TTag {
    pub loaded: Loaded,
    pub id: i64,              // 0
    pub name: Option<String>, // 1
    #[resource(relation)]
    pub author: Lazy<Option<TUser>>, // 2   ToOne, Nullable (nested under TPost::TAGS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, mandate::Action)]
pub enum Action {
    Read,
    Create,
    Update,
    Delete,
    #[action(manage)]
    Manage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, mandate::Subject)]
pub enum Subject {
    #[subject(resource = Post)]
    Post,
    #[subject(resource = Org)]
    Org,
    #[subject(resource = Marker)]
    Marker,
    Dashboard,
    #[subject(all)]
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, mandate::Subject)]
pub enum TSubject {
    #[subject(resource = TPost)]
    TPost,
    #[subject(resource = TOrg)]
    TOrg,
}
