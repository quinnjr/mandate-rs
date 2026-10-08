#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::Ability;

type Ab = Ability<Action, Subject>;

fn ability() -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into()])
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(7))
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .when(Post::STATUS.eq(Status::Draft))
        .build()
        .unwrap()
}

#[test]
fn field_plan_matches_permitted_fields() {
    let ability = ability();
    let fp = ability.field_plan::<Post>(Action::Read).unwrap();
    for author in [7, 8] {
        for status in [Status::Draft, Status::Published] {
            let mut p = post();
            p.author_id = author;
            p.status = status;
            let got = fp.permitted(|i| {
                fp.rules[i]
                    .cond
                    .as_ref()
                    .is_none_or(|c| c.eval(&p).unwrap())
            });
            assert_eq!(got, ability.permitted_fields(Action::Read, &p).unwrap());
        }
    }
}

#[test]
fn rule_order_is_definition_order() {
    let ability = ability();
    let fp = ability.field_plan::<Post>(Action::Read).unwrap();
    let inv: Vec<bool> = fp.rules.iter().map(|r| r.inverted).collect();
    assert_eq!(inv, [false, false, true]);
    assert!(fp.rules[0].cond.is_none());
    assert!(fp.rules[0].fields.is_some());
    assert!(fp.rules[1].cond.is_some());
    assert!(fp.rules[1].fields.is_none());
    assert!(fp.rules[2].cond.is_some());
    // order matters: a later can re-adds what an earlier cannot removed
    let a = Ab::builder()
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    let fp = a.field_plan::<Post>(Action::Read).unwrap();
    assert!(fp.permitted(|_| true).contains(Post::BODY));
}

#[test]
fn clone_and_debug_need_no_bounds() {
    let ability = ability();
    let fp = ability.field_plan::<Post>(Action::Read).unwrap();
    let c = fp.clone();
    assert_eq!(format!("{fp:?}"), format!("{c:?}"));
    assert!(format!("{:?}", fp.rules[0]).contains("FieldRule"));
}
