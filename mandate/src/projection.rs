//! Fetch projections (spec §7.5).

use crate::condition::deps::collect;
use crate::{
    Ability, Action, EvalError, FieldIdx, FieldKind, FieldMask, FieldSet, Schema, Subject,
    SubjectResource,
};

/// What to fetch so the in-memory checks for an action can run: `R`'s scalar and
/// opaque fields, plus the relations the rule conditions read.
#[derive(Debug)]
pub struct Projection<R> {
    /// `R`'s scalar and opaque fields to fetch. Relation fields never appear here.
    pub fields: FieldSet<R>,
    /// Relations read by conditions, sorted by relation index.
    pub relations: Vec<RelationProjection>,
}

impl<R> Clone for Projection<R> {
    fn clone(&self) -> Self {
        Self {
            fields: self.fields,
            relations: self.relations.clone(),
        }
    }
}

/// A relation to load only because a condition reads through it.
#[derive(Clone, Debug)]
pub struct RelationProjection {
    /// The relation field on the owning schema.
    pub relation: FieldIdx,
    /// The relation's target schema.
    pub target: &'static Schema,
    /// The target fields conditions read.
    pub fields: FieldMask,
    /// Relations of the target that conditions read through, sorted by relation index.
    pub relations: Vec<RelationProjection>,
}

impl PartialEq for RelationProjection {
    fn eq(&self, other: &Self) -> bool {
        self.relation == other.relation
            && core::ptr::eq(self.target, other.target)
            && self.fields == other.fields
            && self.relations == other.relations
    }
}

impl<A: Action, S: Subject> Ability<A, S> {
    /// The fields and relations to fetch so that `can`, `can_field` and
    /// `permitted_fields` for `action` never meet unfetched data (§7.5).
    ///
    /// # Examples
    ///
    /// ```
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// let projection = ability.projection::<Post>(Act::Update).unwrap();
    /// // fetch these fields, then run `can`/`permitted_fields` on the loaded row
    /// assert!(projection.fields.contains(Post::AUTHOR_ID));
    /// assert!(projection.relations.is_empty());
    /// ```
    pub fn projection<R: SubjectResource<S>>(&self, action: A) -> Result<Projection<R>, EvalError> {
        Self::guard::<R>()?;
        let schema = R::schema();
        let all = FieldMask::all(schema.fields.len());
        let mut permitted = FieldMask::default();
        let mut fields = FieldMask::default();
        let mut relations = Vec::new();
        for &i in self.cell(action, R::SUBJECT) {
            let rule = &self.rules()[i as usize];
            let listed = rule.fields().unwrap_or(all);
            match (rule.inverted(), rule.condition()) {
                (false, _) => permitted = permitted.union(listed),
                (true, None) => {
                    for f in listed.iter() {
                        permitted.remove(f);
                    }
                }
                (true, Some(_)) => {}
            }
            if let Some(c) = rule.condition() {
                collect(c, schema, &mut fields, &mut relations);
            }
        }
        for f in permitted.iter() {
            if !matches!(
                schema.field(f).map(|d| d.kind),
                Some(FieldKind::Relation { .. })
            ) {
                fields.insert(f);
            }
        }
        Ok(Projection {
            fields: FieldSet::from_mask(fields),
            relations,
        })
    }
}
