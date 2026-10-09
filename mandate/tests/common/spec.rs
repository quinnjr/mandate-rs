// The template example of spec §6.2, on the fixture's `Post`, with one
// context root, `user` (`id`, `org_id`).

/// The spec's example template: `update Post` for its author, on `title`
/// and `body`, with a placeholder, a null test, a to-one and a to-many
/// relation, `$isNull` and `$or`.
pub const SPEC_EXAMPLE: &str = r#"{
  "action": "update",
  "subject": "Post",
  "conditions": {
    "author_id": "${user.id}",
    "status": { "$ne": "archived" },
    "published_at": null,
    "org": { "id": "${user.org_id}" },
    "reviewer": { "$isNull": false },
    "tags": { "$some": { "name": "rust" } },
    "$or": [ { "status": "published" }, { "author_id": "${user.id}" } ]
  },
  "fields": ["title", "body"],
  "inverted": false,
  "reason": "Authors edit their own posts"
}"#;
