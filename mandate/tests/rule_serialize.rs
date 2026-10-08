#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use chrono::{NaiveDate, TimeZone, Utc};
use common::fixture::*;
use mandate::{Ability, Cond, Context, RuleTemplate, Templates};
use uuid::Uuid;

type Ab = Ability<Action, Subject>;

fn round_trip(ability: &Ab) -> String {
    let json = serde_json::to_string(ability.rules()).unwrap();
    let raw: Vec<RuleTemplate> = serde_json::from_str(&json).unwrap();
    let bound = Templates::<Action, Subject>::compile(&raw, &[])
        .unwrap()
        .bind(&Context::empty())
        .unwrap();
    let rebuilt = Ab::builder().extend(bound).build().unwrap();
    assert_eq!(rebuilt.rules(), ability.rules(), "json: {json}");
    json
}

fn one(c: Cond<Post>) -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(c)
        .build()
        .unwrap()
}

#[test]
fn golden_spec_example() {
    let (id, org_id) = (7_i64, 3_i64);
    let ability = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::STATUS.eq(Status::Published))
        .can([Action::Read, Action::Update], Subject::Post)
        .when(Post::AUTHOR_ID.eq(id))
        .fields([Post::TITLE.into(), Post::BODY.into()])
        .can(Action::Read, Subject::Post)
        .when(Post::ORG.then(Org::ID.eq(org_id)))
        .cannot(Action::Delete, Subject::Post)
        .when(Post::STATUS.eq(Status::Published))
        .because("Published posts cannot be deleted")
        .can(Action::Read, Subject::Dashboard)
        .build()
        .unwrap();
    assert_eq!(
        serde_json::to_string(ability.rules()).unwrap(),
        r#"[{"action":"read","subject":"Post","conditions":{"status":"published"}},{"action":"read","subject":"Post","conditions":{"author_id":7},"fields":["title","body"]},{"action":"update","subject":"Post","conditions":{"author_id":7},"fields":["title","body"]},{"action":"read","subject":"Post","conditions":{"org":{"id":3}}},{"action":"delete","subject":"Post","conditions":{"status":"published"},"inverted":true,"reason":"Published posts cannot be deleted"},{"action":"read","subject":"Dashboard"}]"#
    );
    round_trip(&ability);
}

#[test]
fn round_trips_every_node_kind() {
    let dt = Utc.with_ymd_and_hms(2024, 5, 6, 7, 8, 9).unwrap();
    let day = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    let cases: Vec<Cond<Post>> = vec![
        Post::ID.eq(1),
        Post::ID.ne(1),
        Post::SCORE.lt(2.5),
        Post::SCORE.lte(2.5),
        Post::SCORE.gt(2.5),
        Post::SCORE.gte(2.0),
        Post::ID.is_in([1, 2, 3]),
        Post::ID.not_in([1, 2]),
        Post::TITLE.contains("a"),
        Post::TITLE.starts_with("a"),
        Post::TITLE.ends_with("a"),
        Post::LOCKED.eq(true),
        Post::REVIEWER_ID.is_null(),
        Post::REVIEWER_ID.is_not_null(),
        Post::PUBLISHED_AT.gt(dt),
        Post::PUBLISHED_AT.is_in([dt]),
        Post::OWNER.eq(OWNER),
        Post::OWNER.ne(Uuid::nil()),
        Post::OWNER.is_in([OWNER, Uuid::nil(), Uuid::max()]),
        Post::OWNER.not_in([OWNER]),
        Post::DUE.eq(day),
        Post::DUE.ne(day),
        Post::DUE.lt(day),
        Post::DUE.lte(day),
        Post::DUE.gt(day),
        Post::DUE.gte(day),
        // The extremes `YYYY-MM-DD` can represent.
        Post::DUE.is_in([
            day,
            NaiveDate::from_ymd_opt(0, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
        ]),
        Post::DUE.not_in([day]),
        Post::DUE.is_null(),
        Post::DUE.is_not_null(),
        !Post::DUE.lt(day),
        Post::OWNER
            .eq(OWNER)
            .and(Post::DUE.gte(day).or(Post::DUE.is_null())),
        Post::ORG.then(Org::NAME.eq("Acme")),
        Post::REVIEWER.is_null(),
        Post::REVIEWER.is_not_null(),
        Post::REVIEWER.then(User::ID.eq(4)),
        Post::TAGS.some(Tag::ID.eq(1)),
        Post::TAGS.every(Tag::ID.eq(1)),
        Post::TAGS.none(Tag::ID.eq(1)),
        Post::TAGS.some(Tag::NAME.is_null()),
        Post::ID.eq(1).and(Post::AUTHOR_ID.eq(2)),
        Post::SCORE.gt(1.0).and(Post::SCORE.lt(5.0)),
        Post::ID.eq(1).or(Post::AUTHOR_ID.eq(2)),
        Post::ID
            .eq(1)
            .or(Post::ID.eq(2).and(Post::LOCKED.eq(false))),
        !Post::TITLE.contains("x"),
        !(Post::ID.eq(1).and(Post::AUTHOR_ID.eq(2))),
        !Post::ID.eq(1).and(!Post::AUTHOR_ID.eq(2)),
        !Post::TAGS.some(Tag::ID.eq(1)),
        Post::TAGS.some(Tag::ID.eq(1).or(Tag::ID.eq(2))),
    ];
    for c in cases {
        round_trip(&one(c));
    }
}

#[test]
fn relation_shapes() {
    // Non-nullable to-one: Rel{One, None} and Rel{None, None} survive folding.
    for (cond, expected) in [
        (r#"{"org":{}}"#, r#"{"org":{}}"#),
        (r#"{"org":{"$none":{}}}"#, r#"{"org":{"$none":{}}}"#),
        (
            r#"{"org":{"$none":{"id":3}}}"#,
            r#"{"org":{"$none":{"id":3}}}"#,
        ),
    ] {
        let raw: Vec<RuleTemplate> = serde_json::from_str(&format!(
            r#"[{{"action":"read","subject":"Post","conditions":{cond}}}]"#
        ))
        .unwrap();
        let bound = Templates::<Action, Subject>::compile(&raw, &[])
            .unwrap()
            .bind(&Context::empty())
            .unwrap();
        let ability = Ab::builder().extend(bound).build().unwrap();
        assert!(ability.rules()[0].condition().is_some());
        let json = round_trip(&ability);
        assert!(json.contains(expected), "{json}");
    }
}

#[test]
fn dollar_strings_escape() {
    let json = round_trip(&one(Post::TITLE.eq("$x")));
    assert!(json.contains(r#""title":"$$x""#), "{json}");
    let json = round_trip(&one(Post::TITLE.eq("${user.id}")));
    assert!(json.contains(r#""title":"$${user.id}""#), "{json}");
    let json = round_trip(&one(Post::TITLE
        .contains("$a")
        .and(Post::TITLE.is_in(["$b", "c"]))));
    assert!(
        json.contains(r#""$contains":"$$a""#) && json.contains(r#""$$b""#),
        "{json}"
    );
}

#[test]
fn value_encodings() {
    let dt = Utc.with_ymd_and_hms(2024, 5, 6, 7, 8, 9).unwrap();
    let json = round_trip(&one(Post::PUBLISHED_AT.gt(dt)));
    assert!(
        json.contains(r#"{"$gt":"2024-05-06T07:08:09.000000Z"}"#),
        "{json}"
    );
    // UUIDs are hyphenated (lowercase), dates `YYYY-MM-DD` (§6.2).
    let json = round_trip(&one(Post::OWNER.eq(OWNER)));
    assert!(
        json.contains(r#"{"owner":"67e55044-10b1-426f-9247-bb680e5fe0c8"}"#),
        "{json}"
    );
    let json = round_trip(&one(Post::OWNER.is_in([Uuid::nil(), Uuid::max()])));
    assert!(
        json.contains(
            r#"{"$in":["00000000-0000-0000-0000-000000000000","ffffffff-ffff-ffff-ffff-ffffffffffff"]}"#
        ),
        "{json}"
    );
    let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let json = round_trip(&one(Post::DUE.lt(day)));
    assert!(json.contains(r#"{"due":{"$lt":"2026-01-01"}}"#), "{json}");
    let early = NaiveDate::from_ymd_opt(999, 3, 4).unwrap();
    let json = round_trip(&one(Post::DUE.gte(early)));
    assert!(json.contains(r#"{"due":{"$gte":"0999-03-04"}}"#), "{json}");
    let json = round_trip(&one(Post::DUE.is_null()));
    assert!(json.contains(r#"{"due":null}"#), "{json}");
}
