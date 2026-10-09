#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{Resource, SubjectResource};
// The fixture's `Action`/`Subject` enums shadow the traits; import the traits anonymously.
use mandate::{Action as _, Subject as _};

#[test]
fn action_names_and_indices() {
    assert_eq!(Action::Read.name(), "read");
    assert_eq!(Action::from_name("update"), Some(Action::Update));
    assert_eq!(Action::from_name("Update"), None);
    assert_eq!(<Action as mandate::Action>::COUNT, 5);
    assert_eq!(<Action as mandate::Action>::MANAGE, Some(Action::Manage));
    assert_eq!(
        Action::all(),
        &[
            Action::Read,
            Action::Create,
            Action::Update,
            Action::Delete,
            Action::Manage
        ]
    );
    assert_eq!(Action::Delete.index(), 3);
}

#[test]
fn action_rename() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Action)]
    enum A {
        #[action(rename = "publish_now")]
        Publish,
        MultiWord,
    }
    assert_eq!(A::Publish.name(), "publish_now");
    assert_eq!(A::MultiWord.name(), "multi_word");
    assert_eq!(A::from_name("publish_now"), Some(A::Publish));
    assert_eq!(<A as mandate::Action>::MANAGE, None);
}

#[test]
fn subject_names_and_schema() {
    assert_eq!(Subject::Post.name(), "Post");
    assert_eq!(<Subject as mandate::Subject>::ALL, Some(Subject::All));
    assert_eq!(<Subject as mandate::Subject>::COUNT, 5);
    assert_eq!(Subject::from_name("Org"), Some(Subject::Org));
    assert_eq!(Subject::from_name("org"), None);
    assert_eq!(Subject::Dashboard.index(), 3);
    assert_eq!(Subject::all().len(), 5);
    assert!(std::ptr::eq(
        Subject::Post.schema().unwrap(),
        Post::schema()
    ));
    assert!(Subject::Dashboard.schema().is_none() && Subject::All.schema().is_none());
}

#[test]
fn subject_rename() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, mandate::Subject)]
    enum S {
        #[subject(rename = "doc")]
        Document,
    }
    assert_eq!(S::Document.name(), "doc");
    assert_eq!(<S as mandate::Subject>::ALL, None);
}

#[test]
fn subject_resource_binding() {
    assert_eq!(<Post as SubjectResource<Subject>>::SUBJECT, Subject::Post);
    assert_eq!(
        <TPost as SubjectResource<TSubject>>::SUBJECT,
        TSubject::TPost
    );
}
