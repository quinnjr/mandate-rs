// Running code on a thread with a small, fixed stack, the rule set with the
// most `can`/`cannot` alternations `build()` accepts, and the deepest
// condition shapes it accepts. `tests/stack.rs` runs the worst cases this
// way; `tests/limits.rs` checks that one level more is rejected.

use mandate::{Ability, BuildError, Cond, Condition};

use super::fixture::*;

/// The stack size of [`on_small_stack`]: 2 MiB, the default for spawned
/// threads and Tokio workers.
///
/// Measured in a debug build (Linux x86-64, rustc 1.99 and 1.94, bisecting
/// in 4 KiB steps), the worst case is the most alternations with the
/// deepest conditions (`alternating_when(257, …)` over `grouped(64, …)`):
/// it needs about 732 KiB and overflows 4 KiB below that, so 2 MiB leaves
/// about 2.8 times that. The most alternations alone (`alternating(257)`)
/// need about 588 KiB, and the deepest conditions alone about 204 KiB
/// (`related(64)`, the hungriest of the three shapes); the long chains and
/// one-rule-per-record sets run even on the 16 KiB minimum. A release build
/// needs about a fifth of each.
pub const SMALL_STACK: usize = 2 * 1024 * 1024;

/// Runs `f` on a new thread with a [`SMALL_STACK`]-byte stack, named after
/// the calling thread (the test), and returns its result, resuming any panic
/// on the caller's thread.
pub fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name(
            std::thread::current()
                .name()
                .unwrap_or("small-stack")
                .to_owned(),
        )
        .stack_size(SMALL_STACK)
        .spawn(f)
        .expect("spawn a thread")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/// `n` rules on (read, Post) alternating `can`/`cannot`, starting with
/// `can`: rule `i` matches the post with id `i`, so `n - 1` alternations.
/// `alternating(257)` is the most `build()` accepts.
pub fn alternating(n: i64) -> Result<Ability<Action, Subject>, BuildError> {
    alternating_when(n, |i| Post::ID.eq(i))
}

/// [`alternating`] with condition `cond(i)` on rule `i`.
pub fn alternating_when(
    n: i64,
    cond: impl Fn(i64) -> Cond<Post>,
) -> Result<Ability<Action, Subject>, BuildError> {
    let mut b = Ability::builder()
        .can(Action::Read, Subject::Post)
        .when(cond(0));
    for i in 1..n {
        b = if i % 2 == 0 {
            b.can(Action::Read, Subject::Post)
        } else {
            b.cannot(Action::Read, Subject::Post)
        }
        .when(cond(i));
    }
    b.build()
}

/// `n` levels of negation, which folding keeps (`fold` removes double
/// negations, so each `!` is over an `all`): `!all([c, id != i])` for `i`
/// in `0..n / 2`, each over the one before, the innermost over `id == -1`
/// (`!(id == -1)` if `n` is odd).
///
/// `negated(64)` holds on ids -1, 1, 3, …, 31 and nowhere else.
pub fn negated(n: usize) -> Cond<Post> {
    let leaf = Post::ID.eq(-1);
    let leaf = if n % 2 == 1 { !leaf } else { leaf };
    (0i64..)
        .take(n / 2)
        .fold(leaf, |c, i| !Cond::all([c, Post::ID.ne(i)]))
}

/// `n` levels of groups alternating `all`/`any`, which folding keeps: no
/// group has a child of its own kind. Level `i` (from the innermost, over
/// `id == first - 1`) adds `id != first + i` (`all`, `i` even) or `id ==
/// first + i` (`any`, `i` odd).
///
/// It holds on id `first - 1` and on `first + i` for odd `i < n`, and
/// nowhere else.
pub fn grouped(n: usize, first: i64) -> Cond<Post> {
    (0i64..).take(n).fold(Post::ID.eq(first - 1), |c, i| {
        if i % 2 == 0 {
            Cond::all([c, Post::ID.ne(first + i)])
        } else {
            Cond::any([c, Post::ID.eq(first + i)])
        }
    })
}

/// `n` levels alternating a relation and `!`, which folding keeps: no `!`
/// is directly over another. Outermost a `!` if `n` is odd, then post →
/// `reviewer` → user → `!` → `posts` → post → `!` → `reviewer` → …, down to
/// a leaf `id == 1`.
///
/// `related(4 * k)` is `reviewer.then(!posts.some(!related(4 * (k - 1))))`
/// over `related(0) = id == 1`: it holds on a chain of `k` reviewers, each
/// with posts that all satisfy the level below, ending in posts with id 1.
pub fn related(n: usize) -> Cond<Post> {
    /// `n` levels on a post, with a `!` on top if `negate`.
    fn on_post(n: usize, negate: bool) -> Cond<Post> {
        match (n, negate) {
            (0, _) => Post::ID.eq(1),
            (n, true) => !on_post(n - 1, false),
            (n, false) => Post::REVIEWER.then(on_user(n - 1, true)),
        }
    }
    /// `n` levels on a user, with a `!` on top if `negate`.
    fn on_user(n: usize, negate: bool) -> Cond<User> {
        match (n, negate) {
            (0, _) => User::ID.eq(1),
            (n, true) => !on_user(n - 1, false),
            (n, false) => User::POSTS.some(on_post(n - 1, true)),
        }
    }
    on_post(n, n % 2 == 1)
}

/// The nesting depth of `c` as `build()` measures it: every `And`, `Or`,
/// `Not` and `Rel` is one level, and leaves are at depth 0.
pub fn depth(c: &Condition) -> usize {
    match c {
        Condition::And(cs) | Condition::Or(cs) => 1 + cs.iter().map(depth).max().unwrap_or(0),
        Condition::Not(c) => 1 + depth(c),
        Condition::Rel { cond, .. } => 1 + cond.as_deref().map_or(0, depth),
        _ => 0,
    }
}
