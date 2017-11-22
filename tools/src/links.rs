use linkify::{LinkFinder, LinkKind};
use regex::Regex;

pub fn linkify(s: String) -> String {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);
    let mut last = 0;
    let mut result = String::new();
    for link in finder.links(&s) {
        result.push_str(&s[last .. link.start()]);
        result.push_str(&format!("<a href=\"{}\">{}</a>", link.as_str(), link.as_str()));
        last = link.end();
    }
    if last == 0 {
        s
    } else {
        result.push_str(&s[last ..]);
        result
    }
}

pub fn linkify_commit_header(s: &str) -> String {
    lazy_static! {
        static ref BUG_NUMBER_REGEX: Regex = {
            Regex::new(r"\b(?P<bugno>[1-9][0-9]{4,9})\b").unwrap()
        };
        static ref SERVO_PR_REGEX: Regex = {
            Regex::new(r"#(?P<prno>[1-9][0-9]*)\b").unwrap()
        };
    }
    if s.starts_with("servo: ") {
        SERVO_PR_REGEX.replace_all(s, "#<a href=\"https://github.com/servo/servo/pull/$prno\">$prno</a>")
    } else {
        BUG_NUMBER_REGEX.replace_all(s, "<a href=\"https://bugzilla.mozilla.org/show_bug.cgi?id=$bugno\">$bugno</a>")
    }
}

#[test]
fn test_linkify_servo_pr() {
    let linkified =
        linkify_commit_header("servo: Merge #1234 - stylo: Report a specific error for invalid CSS color values (from jdm:valueerr); r=heycam");
    assert!(linkified.contains("github.com"), "{:?}", linkified);
}
