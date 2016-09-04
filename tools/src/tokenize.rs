use std::cell::Cell;
use std;
use std::io::Write;

use languages::LanguageSpec;

#[derive(Debug,PartialEq)]
pub enum TokenKind {
    PlainText,
    Punctuation,
    Identifier(Option<String>),
    StringLiteral,
    Comment,
    RegularExpressionLiteral,
    TagName,
    TagAttrName,
    EndTagName,
    Newline
}

#[derive(Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

fn is_whitespace(ch : char) -> bool {
    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r'
}

pub fn tokenize_plain(string: &String) -> Vec<Token> {
    let lines = string.split('\n');
    let mut tokens = Vec::new();
    let mut start = 0;
    for line in lines {
        if line.len() > 0 {
            tokens.push(Token {
                start: start,
                end: start + line.len(),
                kind: TokenKind::PlainText,
            });
        }
        start += line.len();
        if start == string.len() {
            break;
        }
        tokens.push(Token {
            start: start,
            end: start + 1,
            kind: TokenKind::Newline,
        });
        start += 1;
    }
    tokens
}

pub fn tokenize_c_like(string: &String, spec: &LanguageSpec) -> Vec<Token> {
    fn is_ident(ch: char) -> bool {
        (ch == '_') || ch.is_alphabetic() || ch.is_digit(10)
    }

    let mut tokens = Vec::new();

    let chars : Vec<(usize, char)> = string.char_indices().collect();
    let cur_pos = Cell::new(0);

    let mut next_token_maybe_regexp_literal = true;

    let get_char = || {
        let p = cur_pos.get();
        cur_pos.set(p + 1);
        chars[p]
    };

    let peek_char = || {
        if cur_pos.get() == chars.len() {
            return '!';
        }

        let (_, ch) = chars[cur_pos.get()];
        ch
    };

    let peek_char2 = || {
        if cur_pos.get() + 1 >= chars.len() {
            return '!';
        }

        let (_, ch) = chars[cur_pos.get() + 1];
        ch
    };
    
    let peek_pos = || {
        if cur_pos.get() == chars.len() {
            return string.len();
        }

        let (i, _) = chars[cur_pos.get()];
        i
    };

    while cur_pos.get() < chars.len() {
        let (start, ch) = get_char();
        if is_ident(ch) {
            while is_ident(peek_char()) {
                get_char();
            }

            let word = string[start .. peek_pos()].to_string();
            let class = if spec.reserved_words.contains_key(&word) {
                Some(spec.reserved_words.get(&word).unwrap().clone())
            } else {
                None
            };

            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Identifier(class)});
            next_token_maybe_regexp_literal = word == "return";
        } else if ch == '\n' {
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            // Skip it.
        } else if ch == '#' && spec.hash_comment {
            loop {
                if peek_pos() == string.len() {
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Comment});
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '\n' {
                    break;
                }
            }
            let nl = peek_pos() - 1;
            tokens.push(Token {start: start, end: nl, kind: TokenKind::Comment});
            tokens.push(Token {start: nl, end: peek_pos(), kind: TokenKind::Newline});
        } else if ch == '#' && spec.c_preprocessor {
            while peek_char() == ' ' {
                get_char();
            }

            let id_start = peek_pos();
            while is_ident(peek_char()) {
                get_char();
            }

            let word = "#".to_owned() + &string[id_start .. peek_pos()];
            let class = if spec.reserved_words.contains_key(&word) {
                Some(spec.reserved_words.get(&word).unwrap().clone())
            } else {
                None
            };

            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Identifier(class)});
            next_token_maybe_regexp_literal = false;
        } else if ch == '/' && spec.c_style_comments {
            let ch = peek_char();
            if ch == '*' {
                let mut start = start;
                get_char();
                loop {
                    if peek_pos() == string.len() {
                        writeln!(&mut std::io::stderr(), "Unterminated /* comment").unwrap();
                        return tokens;
                    }

                    let (_, next) = get_char();
                    if next == '*' && peek_char() == '/' {
                        break;
                    } else if next == '\n' {
                        // Tokens shouldn't span across lines.
                        let nl = peek_pos() - 1;
                        if start != nl {
                            tokens.push(Token {start: start, end: nl, kind: TokenKind::Comment});
                        }
                        tokens.push(Token {start: nl, end: peek_pos(), kind: TokenKind::Newline});
                        start = peek_pos();
                    }
                }
                get_char();
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Comment});
            } else if ch == '/' {
                get_char();
                loop {
                    if peek_pos() == string.len() {
                        tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Comment});
                        return tokens;
                    }

                    let (_, next) = get_char();
                    if next == '\n' {
                        break;
                    }
                }
                let nl = peek_pos() - 1;
                tokens.push(Token {start: start, end: nl, kind: TokenKind::Comment});
                tokens.push(Token {start: nl, end: peek_pos(), kind: TokenKind::Newline});
            } else if next_token_maybe_regexp_literal && spec.regexp_literals {
                loop {
                    let (_, next) = get_char();
                    if next == '/' {
                        break;
                    } else if next == '[' {
                        while peek_char() != ']' {
                            get_char();
                        }
                    } else if next == '\\' {
                        get_char();
                    } else if next == '\n' {
                        writeln!(&mut std::io::stderr(), "Invalid regexp literal").unwrap();
                        return tokenize_plain(string);
                    }
                }
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::RegularExpressionLiteral});
                next_token_maybe_regexp_literal = true;
            } else {
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
                next_token_maybe_regexp_literal = true;
            }
        } else if ch == '`' && spec.backtick_strings {
            let mut start = start;
            loop {
                if peek_pos() == string.len() {
                    writeln!(&mut std::io::stderr(), "Unterminated backtick string").unwrap();
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '`' {
                    break;
                } else if next == '\n' {
                    // Tokens shouldn't span across lines.
                    let nl = peek_pos() - 1;
                    if start != nl {
                        tokens.push(Token {start: start, end: nl, kind: TokenKind::StringLiteral});
                    }
                    tokens.push(Token {start: nl, end: peek_pos(), kind: TokenKind::Newline});
                    start = peek_pos();
                } else if next == '\\' {
                    get_char();
                } else if next == '$' && peek_char() == '{' {
                    get_char(); // Skip '{'.
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::StringLiteral});

                    let sub_start = peek_pos();
                    while peek_char() != '}' {
                        if peek_char() == '`' {
                            writeln!(&mut std::io::stderr(), "Nested template string not supported").unwrap();
                            return tokenize_plain(string);
                        }
                        get_char();
                    }

                    let inner = tokenize_c_like(&string[sub_start .. peek_pos()].to_string(), spec);
                    let inner = inner.into_iter().map(
                        |t| Token {start: t.start + sub_start, end: t.end + sub_start, kind: t.kind}
                    );
                    tokens.extend(inner);

                    start = peek_pos();
                    get_char();
                }
            }
            if peek_pos() != start {
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::StringLiteral});
            }
            next_token_maybe_regexp_literal = true;
        } else if ch == '\'' || ch == '"' {
            let need_triple = spec.triple_quote_literals && peek_char() == ch && peek_char2() == ch;
            if need_triple {
                get_char();
                get_char();
            }

            let mut start = start;
            loop {
                if peek_pos() == string.len() {
                    writeln!(&mut std::io::stderr(), "Unterminated quote").unwrap();
                    return tokens;
                }

                let (_, next) = get_char();
                if next == ch && (!need_triple || (peek_char() == ch && peek_char2() == ch)) {
                    if need_triple {
                        get_char();
                        get_char();
                    }
                    break;
                } else if next == '\n' {
                    // Tokens shouldn't span across lines.
                    let nl = peek_pos() - 1;
                    if start != nl {
                        tokens.push(Token {start: start, end: nl, kind: TokenKind::StringLiteral});
                    }
                    tokens.push(Token {start: nl, end: peek_pos(), kind: TokenKind::Newline});
                    start = peek_pos();
                } else if next == '\\' && peek_char() != '\n' {
                    get_char();
                }
            }
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::StringLiteral});
            next_token_maybe_regexp_literal = false;
        } else {
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});

            // Horrible hack to treat '/' in (1+2)/3 as division and not regexp literal.
            let s = string[start .. peek_pos()].to_string();
            next_token_maybe_regexp_literal = s == "=" || s == "(" || s == "{" ||
                s == ":" || s == "&" || s == "|" || s == "!";
        }
    }

    tokens
}

pub fn tokenize_tag_like(string: &String, script_spec: &LanguageSpec) -> Vec<Token> {
    fn is_ident(ch: char) -> bool {
        ch == '.' || ch == '_' || ch == '-' || ch == ':' || ch.is_alphabetic() || ch.is_digit(10)
    }

    let mut tokens = Vec::new();

    let chars : Vec<(usize, char)> = string.char_indices().collect();
    let cur_pos = Cell::new(0);

    fn punctuation_kind(ch: char) -> TokenKind {
        if ch == '\n' {
            TokenKind::Newline
        } else {
            TokenKind::Punctuation
        }
    }

    let get_char = || {
        let p = cur_pos.get();
        cur_pos.set(p + 1);
        chars[p]
    };

    let peek_ahead = |s: &str| {
        let p = cur_pos.get();
        if p + s.len() > chars.len() {
            return false;
        }
        let sub = &chars[p .. p + s.len()];
        let sub = sub.iter().map(|&(_, ch)| ch).collect::<String>();
        return &sub == s;
    };
    
    let peek_pos = || {
        if cur_pos.get() == chars.len() {
            return string.len();
        }

        let (i, _) = chars[cur_pos.get()];
        i
    };

    #[derive(Debug)]
    enum TagState {
        TagNone(usize),
        TagStart(usize),
        TagId(usize),
        TagAfterId,
        TagAttrName(usize),
        TagBeforeEq,
        TagAttrEq,
        TagAttrValue(char, usize),
        TagAttrBareValue(usize),
        EndTagId(usize),
        EndStartTag(usize),
        EndTagDone,
        TagCDATA(usize),
        TagComment(usize),
        TagPI(usize),
        Doctype(bool),
    };

    let mut tag_state = TagState::TagNone(0);

    let mut in_script_tag = false;
    let mut cur_line = 1;
    while cur_pos.get() < chars.len() {
        let (start, ch) = get_char();

        //println!("t {} {:?}", ch, tag_state);

        if ch == '\n' {
            cur_line += 1;
        }
        
        match tag_state {
            TagState::TagNone(plain_start) => {
                let skip = in_script_tag && !peek_ahead("/script");
                if ch == '<' && !skip {
                    if plain_start < start {
                        tokens.push(Token {start: plain_start, end: start, kind: TokenKind::PlainText});
                    }

                    if peek_ahead("!--") {
                        tag_state = TagState::TagComment(start);
                    } else if peek_ahead("![CDATA[") {
                        let _ = get_char(); // !
                        let _ = get_char(); // [
                        let _ = get_char(); // C
                        let _ = get_char(); // D
                        let _ = get_char(); // A
                        let _ = get_char(); // T
                        let _ = get_char(); // A
                        let _ = get_char(); // [
                        tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
                        tag_state = TagState::TagCDATA(peek_pos());
                    } else if peek_ahead("!DOCTYPE") || peek_ahead("!doctype") {
                        tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
                        tag_state = TagState::Doctype(false);
                    } else if peek_ahead("?") {
                        tag_state = TagState::TagPI(start);
                    } else {
                        tag_state = TagState::TagStart(start);
                    }
                } else if ch == '\n' {
                    if plain_start < start {
                        tokens.push(Token {start: plain_start, end: start, kind: TokenKind::PlainText});
                    }
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                    tag_state = TagState::TagNone(peek_pos());
                }
            },
            TagState::TagStart(open) => {
                if is_ident(ch) {
                    tokens.push(Token {start: open, end: start, kind: TokenKind::Punctuation});
                    tag_state = TagState::TagId(start);
                } else if ch == '/' {
                    tokens.push(Token {start: open, end: peek_pos(), kind: TokenKind::Punctuation});
                    tag_state = TagState::EndTagId(peek_pos());
                } else {
                    writeln!(&mut std::io::stderr(), "Error type 1 (line {})", cur_line).unwrap();
                    return tokenize_plain(string);
                }
            },
            TagState::TagId(id_start) => {
                if !is_ident(ch) {
                    tokens.push(Token {start: id_start, end: start, kind: TokenKind::TagName});

                    let word = string[id_start .. start].to_string();
                    in_script_tag = word == "script";

                    if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else {
                        tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
                        if ch == '>' {
                            tag_state = TagState::TagNone(peek_pos());
                        } else if is_whitespace(ch) {
                            tag_state = TagState::TagAfterId;
                        } else {
                            writeln!(&mut std::io::stderr(), "Error type 2 (line {})", cur_line).unwrap();
                            return tokenize_plain(string);
                        }
                    }
                }
            },
            TagState::TagAfterId => {
                if is_ident(ch) {
                    tag_state = TagState::TagAttrName(start);
                } else if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
                } else if ch == '/' {
                    tag_state = TagState::EndStartTag(start);
                } else if is_whitespace(ch) {
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                }
            },
            TagState::TagAttrName(id_start) => {
                if !is_ident(ch) {
                    tokens.push(Token {start: id_start, end: start, kind: TokenKind::TagAttrName});
                    if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else {
                        tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
                        if ch == '>' {
                            tag_state = TagState::TagNone(peek_pos());
                        } else if ch == '=' {
                            tag_state = TagState::TagAttrEq;
                        } else if is_whitespace(ch) {
                            tag_state = TagState::TagBeforeEq;
                        } else {
                            writeln!(&mut std::io::stderr(), "Error type 3 (line {})", cur_line).unwrap();
                            return tokenize_plain(string);
                        }
                    }
                }
            },
            TagState::TagBeforeEq => {
                if ch == '=' {
                    tag_state = TagState::TagAttrEq;
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                } else if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                } else if ch == '/' {
                    tag_state = TagState::EndStartTag(start);
                } else if is_whitespace(ch) {
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                }
            },
            TagState::TagAttrEq => {
                if ch == '"' || ch == '\'' {
                    tag_state = TagState::TagAttrValue(ch, peek_pos());
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation });
                } else if is_whitespace(ch) {
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                } else {
                    tag_state = TagState::TagAttrBareValue(start);
                }
            },
            TagState::TagAttrValue(end_ch, attr_start) => {
                if ch == end_ch {
                    tag_state = TagState::TagAfterId;
                    tokens.push(Token {start: attr_start, end: start, kind: TokenKind::StringLiteral});
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation });
                } else if ch == '\n' {
                    tokens.push(Token {start: attr_start, end: start, kind: TokenKind::StringLiteral});
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                    tag_state = TagState::TagAttrValue(end_ch, peek_pos());
                }
            },
            TagState::TagAttrBareValue(attr_start) => {
                if is_whitespace(ch) || ch == '>' || (ch == '/' && peek_ahead(">")) {
                    tokens.push(Token {start: attr_start, end: start, kind: TokenKind::StringLiteral});
                    if ch == '>' {
                        tag_state = TagState::TagNone(peek_pos());
                        tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
                    } else if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else if is_whitespace(ch) {
                        tag_state = TagState::TagAfterId;
                        tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch) });
                    }
                }
            },
            TagState::EndTagId(id_start) => {
                if !is_ident(ch) {
                    in_script_tag = false;
                    tokens.push(Token {start: id_start, end: start, kind: TokenKind::EndTagName});
                    tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
                    if ch == '>' {
                        tag_state = TagState::TagNone(peek_pos());
                    } else if is_whitespace(ch) {
                        tag_state = TagState::EndTagDone;
                    } else {
                        writeln!(&mut std::io::stderr(), "Error type 4 (line {})", cur_line).unwrap();
                        return tokenize_plain(string);
                    }
                }
            },
            TagState::EndTagDone => {
                if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                } else if !is_whitespace(ch) {
                    writeln!(&mut std::io::stderr(), "Error type 5 (line {})", cur_line).unwrap();
                    return tokenize_plain(string);
                }
                tokens.push(Token {start: start, end: peek_pos(), kind: punctuation_kind(ch)});
            },
            TagState::EndStartTag(slash) => {
                if ch == '>' {
                    in_script_tag = false;
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {start: slash, end: peek_pos(), kind: punctuation_kind(ch)});
                }
            },
            TagState::TagCDATA(cdata_start) => {
                if ch == ']' && peek_ahead("]>") {
                    let _ = get_char();
                    let _ = get_char();
                    
                    if cdata_start < peek_pos() {
                        tokens.push(Token {start: cdata_start, end: peek_pos(), kind: TokenKind::PlainText});
                    }
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    if cdata_start < start {
                        tokens.push(Token {start: cdata_start, end: start, kind: TokenKind::PlainText});
                    }
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                    tag_state = TagState::TagCDATA(peek_pos());
                }
            },
            TagState::TagComment(comment_start) => {
                if ch == '-' && peek_ahead("->") {
                    let _ = get_char();
                    let _ = get_char();
                    tokens.push(Token {start: comment_start, end: peek_pos(), kind: TokenKind::Comment});
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    tokens.push(Token {start: comment_start, end: start, kind: TokenKind::Comment});
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                    tag_state = TagState::TagComment(peek_pos());
                }
            },
            TagState::TagPI(comment_start) => {
                if ch == '>' {
                    tokens.push(Token {start: comment_start, end: peek_pos(), kind: TokenKind::Comment});
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    tokens.push(Token {start: comment_start, end: start, kind: TokenKind::Comment});
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                    tag_state = TagState::TagPI(peek_pos());
                }
            },
            TagState::Doctype(in_bracket) => {
                if ch == '[' && !in_bracket {
                    tag_state = TagState::Doctype(true);
                } else if ch == ']' && in_bracket {
                    tag_state = TagState::Doctype(false);
                } else if ch == '>' && !in_bracket {
                    tag_state = TagState::TagNone(start);
                } else if ch == '<' {
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
                } else if ch == '\n' {
                    tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Newline});
                }
            },
        }
    }

    match tag_state {
        TagState::TagNone(plain_start) => {
            tokens.push(Token {start: plain_start, end: string.len(), kind: TokenKind::PlainText});
        },
        _ => {},
    }

    fn peek(tag_stack: &Vec<&str>, index: usize, check: &str) -> bool {
        if tag_stack.len() <= index {
            return false;
        }
        tag_stack.get(tag_stack.len() - index - 1) == Some(&check)
    }

    let mut result = Vec::new();
    let mut script = String::new();
    let mut tag_stack = Vec::new();
    let mut in_script = false;
    let mut script_start = 0;
    let mut literal_is_id = false;
    let mut literal_is_js = false;
    for token in tokens {
        match token.kind {
            TokenKind::TagName | TokenKind::EndTagName => {
                let tag_name = &string[token.start .. token.end];
                if token.kind == TokenKind::TagName {
                    tag_stack.push(tag_name);
                } else {
                    tag_stack.pop();
                }
                result.push(token);
            },
            TokenKind::TagAttrName => {
                let attr_name = &string[token.start .. token.end];
                if (peek(&tag_stack, 0, "field") ||
                    peek(&tag_stack, 0, "property") ||
                    peek(&tag_stack, 0, "method") ||
                    peek(&tag_stack, 0, "parameter")) &&
                    attr_name == "name"
                {
                    literal_is_id = true;
                }

                if attr_name.starts_with("on") {
                    literal_is_js = true;
                }

                result.push(token);
            },
            TokenKind::PlainText | TokenKind::Newline | TokenKind::Comment => {
                let text = &string[token.start .. token.end];
                if in_script {
                    script.push_str(text);
                } else {
                    result.push(token);
                }
            },
            TokenKind::StringLiteral => {
                if literal_is_id {
                    literal_is_id = false;
                    result.push(Token {start: token.start, end: token.end, kind: TokenKind::Identifier(None)});
                } else if literal_is_js {
                    literal_is_js = false;

                    let script_start = token.start;
                    let script = &string[token.start .. token.end];
                    let script_toks = tokenize_c_like(&script.to_owned(), script_spec);
                    let script_toks = script_toks.into_iter().map(
                        |t| Token {start: t.start + script_start, end: t.end + script_start, kind: t.kind}
                    );
                    result.extend(script_toks);
                } else {
                    result.push(token);
                }
            },
            TokenKind::Punctuation => {
                let punc = &string[token.start .. token.end];

                if punc == "/>" {
                    tag_stack.pop();
                }

                let starting =
                    peek(&tag_stack, 0, "script") ||
                    peek(&tag_stack, 0, "constructor") ||
                    peek(&tag_stack, 0, "destructor") ||
                    peek(&tag_stack, 0, "handler") ||
                    peek(&tag_stack, 0, "field") ||
                    (peek(&tag_stack, 1, "method") && peek(&tag_stack, 0, "body")) ||
                    (peek(&tag_stack, 1, "property") && peek(&tag_stack, 0, "getter")) ||
                    (peek(&tag_stack, 1, "property") && peek(&tag_stack, 0, "setter")) ||
                    false;

                if starting && punc == ">" {
                    in_script = true;
                    script_start = token.end;
                    result.push(token);
                } else if in_script && punc == "</" {
                    let script_toks = tokenize_c_like(&script, script_spec);
                    let script_toks = script_toks.into_iter().map(
                        |t| Token {start: t.start + script_start, end: t.end + script_start, kind: t.kind}
                    );
                    result.extend(script_toks);

                    script = String::new();

                    in_script = false;
                    result.push(token);
                } else if in_script {
                    script.push_str(punc);
                } else {
                    result.push(token);
                }
            },
            _ => {
                assert!(!in_script);
                result.push(token);
            },
        }
    }

    result
}
