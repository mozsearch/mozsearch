use std::cell::Cell;

use languages::LanguageSpec;

#[derive(Debug)]
pub enum TokenKind {
    PlainText,
    Punctuation,
    Identifier(Option<String>),
    StringLiteral,
    Comment,
    RegularExpressionLiteral,
    Newline
}

#[derive(Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

pub fn tokenize_c_like(string: &String, spec: &LanguageSpec) -> Vec<Token> {
    let is_ident = |ch: char| -> bool {
        (ch == '_') || ch.is_alphabetic() || ch.is_digit(10)
    };

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
                        println!("Unterminated /* comment");
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
                        println!("Invalid regexp literal");
                        return tokens;
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
                            println!("Nested template string not supported");
                            return tokens;
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
                    println!("Unterminated quote");
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
                } else if next == '\\' {
                    get_char();
                }
            }
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::StringLiteral});
            next_token_maybe_regexp_literal = false;
        } else {
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});

            // Horrible hack to treat '/' in (1+2)/3 as division and not regexp literal.
            let s = string[start .. peek_pos()].to_string();
            next_token_maybe_regexp_literal = s == "=" || s == "(" || s == "{" || s == ":";
        }
    }

    tokens
}
