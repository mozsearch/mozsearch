//! Escaping for the HTML we generate, whose text and attributes come from the
//! indexed trees (file contents and paths, analysis records' symbols and
//! pretty names, commit messages, ...), none of which may ever become markup
//! or script in the browser.

use std::borrow::Cow;

use serde::Serialize;

/// Escape text for HTML element text or a quoted attribute value (`&`, `<`,
/// `>`, `"` and `'`).
pub fn escape_html(s: &str) -> Cow<'_, str> {
    if !s.contains(['&', '<', '>', '"', '\'']) {
        return Cow::Borrowed(s);
    }
    let mut escaped = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

/// Escape text for a double-quoted attribute value (`&` and `"`, which is all
/// such a value needs, so that its other characters, ex: a C++ symbol's `<`,
/// stay as they are).
pub fn escape_attr(s: &str) -> Cow<'_, str> {
    if !s.contains(['&', '"']) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.replace('&', "&amp;").replace('"', "&quot;"))
}

/// Decode XML's character references (ex: graphviz's SVG's `&#45;` and
/// `&amp;`), leaving any `&` which doesn't start one as it is.
pub fn unescape_xml(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut unescaped = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        unescaped.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let reference = after.find(';').and_then(|semi| {
            let c = match &after[..semi] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                name => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, semi))
        });
        match reference {
            Some((c, semi)) => {
                unescaped.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                unescaped.push('&');
                rest = after;
            }
        }
    }
    unescaped.push_str(rest);
    Cow::Owned(unescaped)
}

/// JSON for a `<script>` element's contents: serde_json's, but with `<`, `>`
/// and `&` (which can only be in its strings) as `\u003c` and so on, so that
/// no string can end the element (`</script>`) or start a comment (`<!--`).
pub fn json_for_script<T: Serialize + ?Sized>(value: &T) -> String {
    escape_json_for_script(&serde_json::to_string(value).unwrap())
}

/// Escape already serialized JSON as `json_for_script` does.
pub fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// A JSON data block (not a script, so a Content-Security-Policy without
/// inline scripts allows it) which static/js/page-data.js makes the global
/// `global` (ex: `SYM_INFO`).
pub fn json_data_block<T: Serialize + ?Sized>(global: &str, value: &T) -> String {
    json_data_block_from_json(global, &serde_json::to_string(value).unwrap())
}

/// A JSON data block of already serialized JSON.
pub fn json_data_block_from_json(global: &str, json: &str) -> String {
    format!(
        r#"<script type="application/json" data-global="{}">{}</script>"#,
        escape_html(global),
        escape_json_for_script(json)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("plain"), "plain");
        assert_eq!(
            escape_html(r#"<img src=x onerror="alert('1')">&"#),
            "&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt;&amp;"
        );
    }

    #[test]
    fn test_escape_attr() {
        assert_eq!(escape_attr("F_<T_a::B>_c"), "F_<T_a::B>_c");
        assert_eq!(
            escape_attr(r#"x"onmouseover="y&z"#),
            "x&quot;onmouseover=&quot;y&amp;z"
        );
    }

    #[test]
    fn test_unescape_xml() {
        assert_eq!(unescape_xml("a&#45;&gt;b"), "a->b");
        assert_eq!(
            unescape_xml("&quot;&apos;&lt;&amp;lt;&#x41;&#66;"),
            "\"'<&lt;AB"
        );
        assert_eq!(unescape_xml("a & b &bogus; &#xZZ;"), "a & b &bogus; &#xZZ;");
    }

    #[test]
    fn test_json_for_script() {
        let value = json!({ "pretty": "</script><script>alert(1)</script><!--", "a&b": 1 });
        let json = json_for_script(&value);
        assert!(!json.contains(['<', '>', '&']), "{json}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            value
        );
    }

    #[test]
    fn test_json_data_block() {
        let value = json!({ "pretty": "</script>" });
        assert_eq!(
            json_data_block("SYM_INFO", &value),
            format!(
                r#"<script type="application/json" data-global="SYM_INFO">{}</script>"#,
                json_for_script(&value)
            )
        );
    }
}
