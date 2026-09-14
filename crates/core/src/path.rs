//! kpcli-style path syntax: `/` separates groups, `\/` is a literal slash,
//! `\\` a literal backslash, `.` and `..` behave like a Unix shell.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Root,
    Up,
    Here,
    Name(String),
}

/// Parse a path spec into segments. An empty spec yields no segments.
pub fn parse(spec: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut chars = spec.chars().peekable();
    if chars.peek() == Some(&'/') {
        segs.push(Segment::Root);
        chars.next();
    }
    let mut cur = String::new();
    let mut escaped = false;
    let flush = |cur: &mut String, segs: &mut Vec<Segment>| {
        match cur.as_str() {
            "" => {}
            "." => segs.push(Segment::Here),
            ".." => segs.push(Segment::Up),
            _ => segs.push(Segment::Name(std::mem::take(cur))),
        }
        cur.clear();
    };
    for c in chars {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '/' {
            flush(&mut cur, &mut segs);
        } else {
            cur.push(c);
        }
    }
    if escaped {
        cur.push('\\');
    }
    flush(&mut cur, &mut segs);
    segs
}

/// Escape a group or entry name for display inside a path.
pub fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            _ => out.push(c),
        }
    }
    out
}

/// Join already-escaped components into an absolute path. Empty input is "/".
pub fn join(components: &[String]) -> String {
    if components.is_empty() {
        "/".to_string()
    } else {
        let mut s = String::new();
        for c in components {
            s.push('/');
            s.push_str(c);
        }
        s
    }
}

/// Split a spec into (parent spec, final partial name) for tab completion.
/// `"/a/b/ge"` becomes `("/a/b/", "ge")`; `"ge"` becomes `("", "ge")`.
pub fn split_for_completion(spec: &str) -> (&str, &str) {
    let bytes = spec.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        if bytes[i - 1] == b'/' {
            let backslashes = bytes[..i - 1]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count();
            if backslashes % 2 == 0 {
                return (&spec[..i], &spec[i..]);
            }
        }
        i -= 1;
    }
    ("", spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Segment::*;

    #[test]
    fn parses_absolute_and_relative() {
        assert_eq!(
            parse("/a/b"),
            vec![Root, Name("a".into()), Name("b".into())]
        );
        assert_eq!(parse("a/b/"), vec![Name("a".into()), Name("b".into())]);
        assert_eq!(parse("../x"), vec![Up, Name("x".into())]);
        assert_eq!(parse("./x"), vec![Here, Name("x".into())]);
        assert_eq!(parse("/"), vec![Root]);
        assert_eq!(parse(""), Vec::<Segment>::new());
        assert_eq!(parse("a//b"), vec![Name("a".into()), Name("b".into())]);
    }

    #[test]
    fn handles_escapes() {
        assert_eq!(
            parse(r"Comcast\/Xfinity"),
            vec![Name("Comcast/Xfinity".into())]
        );
        assert_eq!(parse(r"a\\b"), vec![Name(r"a\b".into())]);
        assert_eq!(escape("Comcast/Xfinity"), r"Comcast\/Xfinity");
        assert_eq!(escape(r"a\b"), r"a\\b");
        assert_eq!(
            parse(&escape("we/ird\\name")),
            vec![Name("we/ird\\name".into())]
        );
    }

    #[test]
    fn joins() {
        assert_eq!(join(&[]), "/");
        assert_eq!(join(&["a".into(), "b".into()]), "/a/b");
    }

    #[test]
    fn splits_for_completion() {
        assert_eq!(split_for_completion("/a/b/ge"), ("/a/b/", "ge"));
        assert_eq!(split_for_completion("ge"), ("", "ge"));
        assert_eq!(split_for_completion("/"), ("/", ""));
        assert_eq!(split_for_completion(r"a\/b"), ("", r"a\/b"));
        assert_eq!(split_for_completion(r"a\\/b"), (r"a\\/", "b"));
    }
}
