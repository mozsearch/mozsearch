use linkify::{LinkFinder, LinkKind};
use regex::Regex;
use std::borrow::Cow;

pub fn linkify_comment(s: String) -> String {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);

    let mut last = 0;
    let mut result = String::new();
    for link in finder.links(&s) {
        result.push_str(&linkify_bug_numbers(&s[last..link.start()]));
        result.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            link.as_str(),
            linkify_bug_numbers(link.as_str())
        ));
        last = link.end();
    }

    if last == 0 {
        return linkify_bug_numbers(&s).into_owned();
    }

    result.push_str(&linkify_bug_numbers(&s[last..]));
    result
}

fn linkify_bug_numbers(s: &str) -> Cow<str> {
    lazy_static! {
        static ref BUG_NUMBER_REGEX: Regex =
            Regex::new(r"\b(?i)bug\s*(?P<bugno>[1-9][0-9]{2,6})\b").unwrap();
    }
    BUG_NUMBER_REGEX.replace_all(
        s,
        "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$0</a>",
    )
}

pub fn linkify_commit_header(s: &str) -> String {
    lazy_static! {
        static ref BUG_NUMBER_REGEX: Regex =
            Regex::new(r"\b(?P<bugno>[1-9][0-9]{4,9})\b").unwrap();
        static ref SERVO_PR_REGEX: Regex = Regex::new(r"#(?P<prno>[1-9][0-9]*)\b").unwrap();
        static ref WPT_SYNC_REGEX: Regex = Regex::new(r"\[wpt PR (?P<prno>[1-9][0-9]*)\]").unwrap();
    }
    if s.starts_with("servo: ") {
        SERVO_PR_REGEX
            .replace_all(
                s,
                "#<a href=\"https://github.com/servo/servo/pull/$prno\">$prno</a>",
            )
            .into_owned()
    } else if s.contains("[wpt PR") {
        // Linkify the "Bug XYZ" bit first. It's slightly unfortunate that this
        // would linkify the whole "Bug <number>" word, but for other commits
        // it'd only linkify the number. It doesn't seem like a big deal though.
        let s = linkify_bug_numbers(s);
        WPT_SYNC_REGEX
            .replace_all(
                &*s,
                r#"[<a href="https://github.com/web-platform-tests/wpt/pull/$prno">wpt PR $prno</a>]"#,
            )
            .into_owned()
    } else {
        BUG_NUMBER_REGEX
            .replace_all(
                s,
                "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$bugno</a>",
            )
            .into_owned()
    }
}

#[test]
fn test_linkify_wpt_sync() {
    let linkified =
        linkify_commit_header("Bug 1643934 [wpt PR 24024] - Align prefers-color-scheme:no-preference tests with spec., a=testonly");
    assert_eq!(
        linkified,
        r#"<a href="https://bugzilla.mozilla.org/show_bug.cgi?id=1643934">Bug 1643934</a> [<a href="https://github.com/web-platform-tests/wpt/pull/24024">wpt PR 24024</a>] - Align prefers-color-scheme:no-preference tests with spec., a=testonly"#,
    );
}

#[test]
fn test_linkify_servo_pr() {
    let linkified =
        linkify_commit_header("servo: Merge #1234 - stylo: Report a specific error for invalid CSS color values (from jdm:valueerr); r=heycam");
    assert!(linkified.contains("github.com"), "{:?}", linkified);
}

#[test]
fn test_bug_number() {
    let linkified = linkify_bug_numbers("this is a link to bUg 12345");
    assert!(
        linkified.contains("bugzilla.mozilla.org"),
        "{:?}",
        linkified
    );
    assert!(linkified.contains(">bUg 12345</a>"), "{:?}", linkified);
}

#[test]
fn test_bug_number_inside_link() {
    let link = "http://example.org/browser/editor/libeditor/tests/bug629172.html";
    let linkified = linkify_comment(link.into());
    assert_eq!(linkified, "<a href=\"http://example.org/browser/editor/libeditor/tests/bug629172.html\">http://example.org/browser/editor/libeditor/tests/<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=629172\">bug629172</a>.html</a>");
}
