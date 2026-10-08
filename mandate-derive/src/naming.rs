//! Serde-compatible renaming rules and identifier names, shared by the derives.

use syn::Ident;

/// An identifier as written, without its raw-identifier prefix (`r#type` is
/// `type`).
pub(crate) fn unraw(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("r#").map(str::to_owned).unwrap_or(s)
}

/// Splits a Rust identifier (PascalCase or snake_case) into lowercase words.
fn words(ident: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in ident.chars() {
        if c == '_' {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
        } else if c.is_uppercase() {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
            cur.extend(c.to_lowercase());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn capitalize(w: &str) -> String {
    let mut cs = w.chars();
    match cs.next() {
        Some(f) => f.to_uppercase().chain(cs).collect(),
        None => String::new(),
    }
}

/// Applies a serde `rename_all` rule to an identifier as written in the source.
pub(crate) fn apply_rename_all(rule: &str, ident: &str) -> Result<String, String> {
    let w = words(ident);
    Ok(match rule {
        "lowercase" => ident.to_lowercase(),
        "UPPERCASE" => ident.to_uppercase(),
        "PascalCase" => w.iter().map(|x| capitalize(x)).collect(),
        "camelCase" => {
            let mut s = String::new();
            for (i, x) in w.iter().enumerate() {
                if i == 0 {
                    s.push_str(x);
                } else {
                    s.push_str(&capitalize(x));
                }
            }
            s
        }
        "snake_case" => w.join("_"),
        "SCREAMING_SNAKE_CASE" => w.join("_").to_uppercase(),
        "kebab-case" => w.join("-"),
        "SCREAMING-KEBAB-CASE" => w.join("-").to_uppercase(),
        other => return Err(format!("unknown rename_all rule `{other}`")),
    })
}

#[cfg(test)]
mod tests {
    use super::apply_rename_all as r;
    use super::unraw;
    use proc_macro2::Span;
    use syn::Ident;

    #[test]
    fn unraw_strips_only_the_raw_prefix() {
        let raw = |s| unraw(&Ident::new_raw(s, Span::call_site()));
        assert_eq!(raw("match"), "match");
        assert_eq!(raw("LoopAround"), "LoopAround");
        assert_eq!(unraw(&Ident::new("plain", Span::call_site())), "plain");
    }

    #[test]
    fn rules() {
        assert_eq!(r("lowercase", "FooBar").unwrap(), "foobar");
        assert_eq!(r("UPPERCASE", "FooBar").unwrap(), "FOOBAR");
        assert_eq!(r("PascalCase", "foo_bar").unwrap(), "FooBar");
        assert_eq!(r("camelCase", "FooBar").unwrap(), "fooBar");
        assert_eq!(r("snake_case", "FooBar").unwrap(), "foo_bar");
        assert_eq!(r("SCREAMING_SNAKE_CASE", "FooBar").unwrap(), "FOO_BAR");
        assert_eq!(r("kebab-case", "FooBar").unwrap(), "foo-bar");
        assert_eq!(r("SCREAMING-KEBAB-CASE", "FooBar").unwrap(), "FOO-BAR");
        assert!(r("nope", "x").is_err());
    }
}
