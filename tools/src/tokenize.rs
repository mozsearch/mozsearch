use std::cell::Cell;

use crate::languages::LanguageSpec;

#[derive(Clone, Debug, PartialEq)]
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
    Newline,
}

#[derive(Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

fn is_whitespace(ch: char) -> bool {
    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r'
}

pub fn tokenize_css(string: &str) -> Vec<Token> {
    fn tokenize_css_block(input: &mut cssparser::Parser, raw_input: &str, tokens: &mut Vec<Token>) {
        use cssparser::Token::*;
        let reserved = crate::languages::SYN_RESERVED_CLASS;
        let mut start = input.position().byte_index();
        while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
            let mut has_block = false;
            let kind = match token {
                Ident(name) => {
                    // Poor heuristic to try to find property names.
                    let state = input.state();
                    let is_custom_property = name.starts_with("--");
                    let colon_and_space_follows =
                        matches!(input.next_including_whitespace_and_comments(), Ok(&Colon))
                            && matches!(
                                input.next_including_whitespace_and_comments(),
                                Ok(&WhiteSpace(..))
                            );
                    input.reset(&state);
                    TokenKind::Identifier(if !is_custom_property && colon_and_space_follows {
                        Some(reserved.into())
                    } else {
                        None
                    })
                }
                AtKeyword(..) => TokenKind::Identifier(Some(reserved.into())),
                IDHash(..) | Hash(..) => TokenKind::Identifier(None),
                QuotedString(..) => TokenKind::StringLiteral,
                Colon | Semicolon | Comma | IncludeMatch | DashMatch | PrefixMatch
                | SuffixMatch | SubstringMatch | CloseParenthesis | CloseSquareBracket
                | CloseCurlyBracket | Delim(..) => TokenKind::Punctuation,
                BadUrl(..)
                | BadString(..)
                | UnquotedUrl(..)
                | Number { .. }
                | Percentage { .. }
                | Dimension { .. }
                | WhiteSpace(..) => TokenKind::PlainText,
                CDO | CDC | Comment(..) => TokenKind::Comment,
                Function(..) => {
                    has_block = true;
                    TokenKind::Identifier(None)
                }
                ParenthesisBlock | SquareBracketBlock | CurlyBracketBlock => {
                    has_block = true;
                    TokenKind::Punctuation
                }
            };

            fn push_tokens(
                raw_input: &str,
                start: usize,
                end: usize,
                kind: &TokenKind,
                tokens: &mut Vec<Token>,
            ) {
                if start == end {
                    return;
                }
                // tokens shouldn't span across lines
                let mut span_start = start;
                for span in raw_input[start..end].split('\n') {
                    let span_end = span_start + span.len();
                    if span_start != span_end {
                        tokens.push(Token {
                            start: span_start,
                            end: span_end,
                            kind: kind.clone(),
                        });
                    }
                    let newline_needed = span_start + span.len() != end;
                    span_start = span_end;
                    if newline_needed {
                        tokens.push(Token {
                            start: span_start,
                            end: span_start + 1,
                            kind: TokenKind::Newline,
                        });
                        span_start += 1;
                    }
                }
            }

            if has_block {
                let mut block_start = start;
                let mut block_end = start;
                let mut block_tokens = vec![];
                let _: Result<(), cssparser::ParseError<()>> = input.parse_nested_block(|input| {
                    block_start = input.position().byte_index();
                    tokenize_css_block(input, raw_input, &mut block_tokens);
                    block_end = input.position().byte_index();
                    Ok(())
                });
                push_tokens(raw_input, start, block_start, &kind, tokens);
                tokens.extend(block_tokens.into_iter());
                let end = input.position().byte_index();
                push_tokens(raw_input, block_end, end, &kind, tokens);
                start = end;
            } else {
                let end = input.position().byte_index();
                push_tokens(raw_input, start, end, &kind, tokens);
                start = end;
            }
        }
    }

    let mut input = cssparser::ParserInput::new(string);
    let mut input = cssparser::Parser::new(&mut input);
    let mut tokens = vec![];

    tokenize_css_block(&mut input, string, &mut tokens);

    tokens
}

pub fn tokenize_plain(string: &str) -> Vec<Token> {
    let lines = string.split('\n');
    let mut tokens = Vec::new();
    let mut start = 0;
    for line in lines {
        if line.len() > 0 {
            tokens.push(Token {
                start,
                end: start + line.len(),
                kind: TokenKind::PlainText,
            });
        }
        start += line.len();
        if start == string.len() {
            break;
        }
        tokens.push(Token {
            start,
            end: start + 1,
            kind: TokenKind::Newline,
        });
        start += 1;
    }
    tokens
}

pub fn tokenize_c_like(string: &str, spec: &LanguageSpec) -> Vec<Token> {
    let is_ident = |ch: char| -> bool {
        (ch == '_') || ch.is_alphabetic() || ch.is_digit(10) || (ch == '#' && spec.hash_identifier)
    };

    let mut tokens = Vec::new();

    let chars: Vec<(usize, char)> = string.char_indices().collect();
    let mut backtick_nesting: Vec<Cell<u32>> = Vec::new();
    let cur_pos = Cell::new(0);

    let mut next_token_maybe_regexp_literal = true;

    let get_char = || {
        let p = cur_pos.get();
        // Defense in depth bailing if we would otherwise throw on chars[p]
        // below.
        if p == chars.len() {
            // We return '!' in peek cases past the end for reasons that aren't
            // immediately clear.  I've gone with a nul here because this seems
            // less likely to result in the state machine advancing into a
            // state that doesn't correspond with reality, but wouldn't be
            // surprised if this turns out to have its own problems.
            debug!("Attempted read past end");
            return (p, '\0');
        }
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

    let peek_isnot = |c| {
        if cur_pos.get() == chars.len() {
            return false;
        }

        let (_, ch) = chars[cur_pos.get()];
        ch != c
    };

    let peek_pos = || {
        if cur_pos.get() == chars.len() {
            return string.len();
        }

        let (i, _) = chars[cur_pos.get()];
        i
    };

    // Breaks the current token across a newline to keep line numbering
    // correct. Returns the new value for `start` on the new line.
    let push_newline = |start: usize, tokens: &mut Vec<_>, cur_tok_kind: TokenKind| -> usize {
        let nl = peek_pos() - 1;
        if start != nl {
            tokens.push(Token {
                start,
                end: nl,
                kind: cur_tok_kind,
            });
        }
        tokens.push(Token {
            start: nl,
            end: peek_pos(),
            kind: TokenKind::Newline,
        });
        nl + 1
    };

    'token_loop: while cur_pos.get() < chars.len() {
        let (start, mut ch) = get_char();
        let mut continue_backtick = false;
        if let Some(braces) = backtick_nesting.last_mut() {
            match ch {
                '{' => {
                    braces.set(braces.get() + 1);
                }
                '}' => {
                    braces.set(braces.get() - 1);
                    continue_backtick = braces.get() == 0;
                }
                _ => {}
            }
        }

        // If continue_backtick is true, then backtick_nesting.last() is
        // non-None and 0, so pop it off the stack before continuing.
        if continue_backtick {
            backtick_nesting.pop();
        }

        // Pre-process rust byte strings here. To do so, scan ahead a little:
        // - If the next character is not a quote or an r, this is an
        //   identifier. No action is taken.
        // - If the next character is a quote, we have a byte string.
        // - If the next character is an r, check the following character:
        //   - If that character is a # or a quote, then this is a raw byte
        //     string literal.
        // In the cases where we don't have a byte string, we do nothing.
        // Otherwise, consume the 'b', but leave `start` alone. This way, 'ch'
        // will point to the proper character to consume this token (either
        // the 'r' for a raw string literal or a quote for a byte string).
        if spec.rust_tweaks && ch == 'b' {
            match (peek_char(), peek_char2()) {
                ('\'', _) | ('"', _) | ('r', '"') | ('r', '#') => {
                    let (_, next) = get_char();
                    ch = next;
                }
                _ => {}
            }
        }

        if spec.rust_tweaks && ch == 'r' && (peek_char() == '#' || peek_char() == '"') {
            // Rust raw string literals.
            // Consume 0 or more #s.
            let mut nhashes = 0;
            loop {
                let (_, ch) = get_char();
                if ch == '"' {
                    break;
                } else if ch != '#' || peek_pos() == string.len() {
                    // Not actually a (valid) Rust raw string literal. We can
                    // run into this in macro inputs or rust files that are
                    // intentionally syntactically invalid (as can happen in the
                    // rust tree).
                    //
                    // Just treat it as plain text and move on, which is
                    // better than crashing
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::PlainText,
                    });
                    continue 'token_loop;
                }
                nhashes += 1;
            }

            let mut start = start;
            'rust_raw_string: loop {
                if peek_pos() == string.len() {
                    debug!("Unterminated raw string");
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '\n' {
                    // Tokens shouldn't span across lines.
                    start = push_newline(start, &mut tokens, TokenKind::StringLiteral);
                }
                if next != '"' {
                    continue;
                }

                // Consume nhashes #s.
                for _ in 0..nhashes {
                    if peek_char() != '#' {
                        continue 'rust_raw_string;
                    }
                    get_char();
                }

                break;
            }
            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::StringLiteral,
            });
        } else if ch == 'R' && peek_char() == '"' {
            // Handle raw literals per
            // <http://en.cppreference.com/w/cpp/language/string_literal>.
            get_char();

            // Read the delimiter.
            let paren;
            loop {
                let (idx, c) = get_char();
                if c == '(' {
                    paren = idx;
                    break;
                }

                if peek_pos() == string.len() {
                    debug!("Expecting '(' after raw string literal");
                    return tokens;
                }
            }

            let delimiter = &string[start + 2..paren];

            let mut start = start;
            'raw_string: loop {
                if peek_pos() == string.len() {
                    debug!("Unterminated raw string");
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '\n' {
                    // Tokens shouldn't span across lines.
                    start = push_newline(start, &mut tokens, TokenKind::StringLiteral);
                }
                if next != ')' {
                    continue;
                }

                // Find the delimiter.
                for c in delimiter.chars() {
                    if c != peek_char() {
                        continue 'raw_string;
                    }
                    get_char();
                }

                // Is this the end quote?
                if peek_char() != '"' {
                    continue;
                }
                get_char();

                // Done!
                break;
            }

            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::StringLiteral,
            });
            next_token_maybe_regexp_literal = false;
        } else if is_ident(ch) {
            let cxx14_number = spec.cxx14_digit_separators && ch.is_digit(10);
            while is_ident(peek_char()) || (cxx14_number && peek_char() == '\'') {
                get_char();
            }

            let word = string[start..peek_pos()].to_string();
            let class = spec.reserved_words.get(&word).cloned();

            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::Identifier(class),
            });
            next_token_maybe_regexp_literal = word == "return";
        } else if ch == '\n' {
            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::Newline,
            });
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            // Skip it.
        } else if ch == '#' && spec.hash_comment {
            loop {
                if peek_pos() == string.len() {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Comment,
                    });
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '\n' {
                    break;
                }
            }
            let nl = peek_pos() - 1;
            tokens.push(Token {
                start,
                end: nl,
                kind: TokenKind::Comment,
            });
            tokens.push(Token {
                start: nl,
                end: peek_pos(),
                kind: TokenKind::Newline,
            });
        } else if ch == '#' && spec.c_preprocessor {
            while peek_char() == ' ' || peek_char() == '\t' {
                get_char();
            }

            let id_start = peek_pos();
            while is_ident(peek_char()) {
                get_char();
            }

            let word = "#".to_owned() + &string[id_start..peek_pos()];
            let class = spec.reserved_words.get(&word).cloned();

            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::Identifier(class),
            });
            next_token_maybe_regexp_literal = false;
        } else if ch == '/' && spec.c_style_comments {
            let ch = peek_char();
            if ch == '*' {
                let mut nesting = 1;
                let mut start = start;
                get_char();
                loop {
                    if peek_pos() == string.len() {
                        debug!("Unterminated /* comment");
                        return tokens;
                    }

                    let (_, next) = get_char();
                    if next == '*' && peek_char() == '/' {
                        if nesting == 1 {
                            break;
                        }
                        get_char();
                        nesting -= 1;
                    } else if next == '\n' {
                        // Tokens shouldn't span across lines.
                        start = push_newline(start, &mut tokens, TokenKind::Comment);
                    } else if spec.rust_tweaks && next == '/' && peek_char() == '*' {
                        get_char();
                        nesting += 1;
                    }
                }
                get_char();
                tokens.push(Token {
                    start,
                    end: peek_pos(),
                    kind: TokenKind::Comment,
                });
            } else if ch == '/' {
                get_char();
                loop {
                    if peek_pos() == string.len() {
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: TokenKind::Comment,
                        });
                        return tokens;
                    }

                    let (_, next) = get_char();
                    if next == '\n' {
                        break;
                    }
                }
                let nl = peek_pos() - 1;
                tokens.push(Token {
                    start,
                    end: nl,
                    kind: TokenKind::Comment,
                });
                tokens.push(Token {
                    start: nl,
                    end: peek_pos(),
                    kind: TokenKind::Newline,
                });
            } else if next_token_maybe_regexp_literal && spec.regexp_literals {
                loop {
                    if cur_pos.get() == chars.len() {
                        debug!("Invalid regexp literal");
                        return tokenize_plain(string);
                    }
                    let (_, next) = get_char();
                    if next == '/' {
                        break;
                    } else if next == '[' {
                        while peek_isnot(']') {
                            get_char();
                        }
                    } else if next == '\\' && peek_isnot('\n') {
                        get_char();
                    } else if next == '\n' {
                        debug!("Invalid regexp literal");
                        return tokenize_plain(string);
                    }
                }
                tokens.push(Token {
                    start,
                    end: peek_pos(),
                    kind: TokenKind::RegularExpressionLiteral,
                });
                next_token_maybe_regexp_literal = true;
            } else {
                tokens.push(Token {
                    start,
                    end: peek_pos(),
                    kind: TokenKind::Punctuation,
                });
                next_token_maybe_regexp_literal = true;
            }
        } else if continue_backtick || (ch == '`' && spec.backtick_strings) {
            let mut start = start;
            loop {
                if peek_pos() == string.len() {
                    debug!("Unterminated backtick string");
                    return tokens;
                }

                let (_, next) = get_char();
                if next == '`' {
                    break;
                } else if next == '\n' {
                    // Tokens shouldn't span across lines.
                    start = push_newline(start, &mut tokens, TokenKind::StringLiteral);
                } else if next == '\\' && peek_isnot('\n') {
                    get_char();
                } else if next == '$' && peek_char() == '{' {
                    // A template! Note that we're in a template and start
                    // counting unconsumed { and } tokens. When we find the
                    // last close brace that isn't part of a string or regexp,
                    // we'll come back in here to finish this template string.
                    get_char(); // Skip '{'.
                    backtick_nesting.push(Cell::new(1));
                    break;
                }
            }
            if peek_pos() != start {
                tokens.push(Token {
                    start,
                    end: peek_pos(),
                    kind: TokenKind::StringLiteral,
                });
            }
            next_token_maybe_regexp_literal = true;
        } else if ch == '\'' || ch == '"' {
            let need_triple = spec.triple_quote_literals && peek_char() == ch && peek_char2() == ch;
            if need_triple {
                get_char();
                get_char();
            }

            // In Rust, ' could be the start of a byte literal *or* a
            // Lisp-like atom. Check for that here (but be careful for
            // something like '\n').
            if spec.rust_tweaks && ch == '\'' && peek_char() != '\\' && peek_char2() != '\'' {
                // Push the lonely quote.
                tokens.push(Token {
                    start,
                    end: start + 1,
                    kind: TokenKind::Punctuation,
                });
                loop {
                    if peek_pos() == string.len() {
                        break;
                    }

                    if !is_ident(peek_char()) {
                        break;
                    }
                    get_char();
                }

                // Push the rest of the label.
                tokens.push(Token {
                    start: start + 1,
                    end: peek_pos(),
                    kind: TokenKind::Identifier(None),
                });
                continue;
            }

            let mut start = start;
            loop {
                if peek_pos() == string.len() {
                    debug!("Unterminated quote");
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
                    start = push_newline(start, &mut tokens, TokenKind::StringLiteral);
                } else if next == '\\' && peek_isnot('\n') {
                    get_char();
                }
            }
            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::StringLiteral,
            });
            next_token_maybe_regexp_literal = false;
        } else {
            tokens.push(Token {
                start,
                end: peek_pos(),
                kind: TokenKind::Punctuation,
            });

            // Horrible hack to treat '/' in (1+2)/3 as division and not regexp literal.
            let s = string[start..peek_pos()].to_string();
            next_token_maybe_regexp_literal = s == "="
                || s == "("
                || s == "{"
                || s == ":"
                || s == "&"
                || s == "|"
                || s == "!"
                || s == ","
                || s == "?"
                || s == ">"
                || s == "<";
        }
    }

    tokens
}

pub fn tokenize_tag_like(string: &str, script_spec: &LanguageSpec) -> Vec<Token> {
    fn is_ident(ch: char) -> bool {
        ch == '.' || ch == '_' || ch == '-' || ch == ':' || ch.is_alphabetic() || ch.is_digit(10)
    }

    let mut tokens = Vec::new();

    let chars: Vec<(usize, char)> = string.char_indices().collect();
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
        let sub = &chars[p..p + s.len()];
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
    }

    let mut tag_state = TagState::TagNone(0);

    let mut in_script_tag = false;
    let mut in_style_tag = false;
    let mut cur_line = 1;
    while cur_pos.get() < chars.len() {
        let (start, ch) = get_char();

        //println!("t {} {:?}", ch, tag_state);

        if ch == '\n' {
            cur_line += 1;
        }

        match tag_state {
            TagState::TagNone(plain_start) => {
                let skip = (in_script_tag && !peek_ahead("/script")) ||
                    (in_style_tag && !peek_ahead("/style"));
                if ch == '<' && !skip {
                    if plain_start < start {
                        tokens.push(Token {
                            start: plain_start,
                            end: start,
                            kind: TokenKind::PlainText,
                        });
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
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: TokenKind::Punctuation,
                        });
                        tag_state = TagState::TagCDATA(peek_pos());
                    } else if peek_ahead("!DOCTYPE") || peek_ahead("!doctype") {
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: TokenKind::Punctuation,
                        });
                        tag_state = TagState::Doctype(false);
                    } else if peek_ahead("?") {
                        tag_state = TagState::TagPI(start);
                    } else {
                        tag_state = TagState::TagStart(start);
                    }
                } else if ch == '\n' {
                    if plain_start < start {
                        tokens.push(Token {
                            start: plain_start,
                            end: start,
                            kind: TokenKind::PlainText,
                        });
                    }
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                    tag_state = TagState::TagNone(peek_pos());
                }
            }
            TagState::TagStart(open) => {
                if is_ident(ch) {
                    tokens.push(Token {
                        start: open,
                        end: start,
                        kind: TokenKind::Punctuation,
                    });
                    tag_state = TagState::TagId(start);
                } else if ch == '/' {
                    tokens.push(Token {
                        start: open,
                        end: peek_pos(),
                        kind: TokenKind::Punctuation,
                    });
                    tag_state = TagState::EndTagId(peek_pos());
                } else {
                    debug!("Error type 1 (line {})", cur_line);
                    return tokenize_plain(string);
                }
            }
            TagState::TagId(id_start) => {
                if !is_ident(ch) {
                    tokens.push(Token {
                        start: id_start,
                        end: start,
                        kind: TokenKind::TagName,
                    });

                    let word = string[id_start..start].to_string();
                    in_script_tag = word == "script";
                    in_style_tag = word == "style";

                    if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else {
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: punctuation_kind(ch),
                        });
                        if ch == '>' {
                            tag_state = TagState::TagNone(peek_pos());
                        } else if is_whitespace(ch) {
                            tag_state = TagState::TagAfterId;
                        } else {
                            debug!("Error type 2 (line {})", cur_line);
                            return tokenize_plain(string);
                        }
                    }
                }
            }
            TagState::TagAfterId => {
                if is_ident(ch) {
                    tag_state = TagState::TagAttrName(start);
                } else if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                } else if ch == '/' {
                    tag_state = TagState::EndStartTag(start);
                } else if is_whitespace(ch) {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                }
            }
            TagState::TagAttrName(id_start) => {
                if !is_ident(ch) {
                    tokens.push(Token {
                        start: id_start,
                        end: start,
                        kind: TokenKind::TagAttrName,
                    });
                    if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else {
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: punctuation_kind(ch),
                        });
                        if ch == '>' {
                            tag_state = TagState::TagNone(peek_pos());
                        } else if ch == '=' {
                            tag_state = TagState::TagAttrEq;
                        } else if is_whitespace(ch) {
                            tag_state = TagState::TagBeforeEq;
                        } else {
                            debug!("Error type 3 (line {})", cur_line);
                            return tokenize_plain(string);
                        }
                    }
                }
            }
            TagState::TagBeforeEq => {
                if ch == '=' {
                    tag_state = TagState::TagAttrEq;
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                } else if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                } else if ch == '/' {
                    tag_state = TagState::EndStartTag(start);
                } else if is_whitespace(ch) {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                }
            }
            TagState::TagAttrEq => {
                if ch == '"' || ch == '\'' {
                    tag_state = TagState::TagAttrValue(ch, peek_pos());
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Punctuation,
                    });
                } else if is_whitespace(ch) {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                } else {
                    tag_state = TagState::TagAttrBareValue(start);
                }
            }
            TagState::TagAttrValue(end_ch, attr_start) => {
                if ch == end_ch {
                    tag_state = TagState::TagAfterId;
                    tokens.push(Token {
                        start: attr_start,
                        end: start,
                        kind: TokenKind::StringLiteral,
                    });
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Punctuation,
                    });
                } else if ch == '\n' {
                    tokens.push(Token {
                        start: attr_start,
                        end: start,
                        kind: TokenKind::StringLiteral,
                    });
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                    tag_state = TagState::TagAttrValue(end_ch, peek_pos());
                }
            }
            TagState::TagAttrBareValue(attr_start) => {
                if is_whitespace(ch) || ch == '>' || (ch == '/' && peek_ahead(">")) {
                    tokens.push(Token {
                        start: attr_start,
                        end: start,
                        kind: TokenKind::StringLiteral,
                    });
                    if ch == '>' {
                        tag_state = TagState::TagNone(peek_pos());
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: punctuation_kind(ch),
                        });
                    } else if ch == '/' {
                        tag_state = TagState::EndStartTag(start);
                    } else if is_whitespace(ch) {
                        tag_state = TagState::TagAfterId;
                        tokens.push(Token {
                            start,
                            end: peek_pos(),
                            kind: punctuation_kind(ch),
                        });
                    }
                }
            }
            TagState::EndTagId(id_start) => {
                if !is_ident(ch) {
                    in_script_tag = false;
                    in_style_tag = false;
                    tokens.push(Token {
                        start: id_start,
                        end: start,
                        kind: TokenKind::EndTagName,
                    });
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                    if ch == '>' {
                        tag_state = TagState::TagNone(peek_pos());
                    } else if is_whitespace(ch) {
                        tag_state = TagState::EndTagDone;
                    } else {
                        debug!("Error type 4 (line {})", cur_line);
                        return tokenize_plain(string);
                    }
                }
            }
            TagState::EndTagDone => {
                if ch == '>' {
                    tag_state = TagState::TagNone(peek_pos());
                } else if !is_whitespace(ch) {
                    debug!("Error type 5 (line {})", cur_line);
                    return tokenize_plain(string);
                }
                tokens.push(Token {
                    start,
                    end: peek_pos(),
                    kind: punctuation_kind(ch),
                });
            }
            TagState::EndStartTag(slash) => {
                if ch == '>' {
                    in_script_tag = false;
                    in_style_tag = false;
                    tag_state = TagState::TagNone(peek_pos());
                    tokens.push(Token {
                        start: slash,
                        end: peek_pos(),
                        kind: punctuation_kind(ch),
                    });
                }
            }
            TagState::TagCDATA(cdata_start) => {
                if ch == ']' && peek_ahead("]>") {
                    let _ = get_char();
                    let _ = get_char();

                    if cdata_start < peek_pos() {
                        tokens.push(Token {
                            start: cdata_start,
                            end: peek_pos(),
                            kind: TokenKind::PlainText,
                        });
                    }
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    if cdata_start < start {
                        tokens.push(Token {
                            start: cdata_start,
                            end: start,
                            kind: TokenKind::PlainText,
                        });
                    }
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                    tag_state = TagState::TagCDATA(peek_pos());
                }
            }
            TagState::TagComment(comment_start) => {
                if ch == '-' && peek_ahead("->") {
                    let _ = get_char();
                    let _ = get_char();
                    tokens.push(Token {
                        start: comment_start,
                        end: peek_pos(),
                        kind: TokenKind::Comment,
                    });
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    tokens.push(Token {
                        start: comment_start,
                        end: start,
                        kind: TokenKind::Comment,
                    });
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                    tag_state = TagState::TagComment(peek_pos());
                }
            }
            TagState::TagPI(comment_start) => {
                if ch == '>' {
                    tokens.push(Token {
                        start: comment_start,
                        end: peek_pos(),
                        kind: TokenKind::Comment,
                    });
                    tag_state = TagState::TagNone(peek_pos());
                } else if ch == '\n' {
                    tokens.push(Token {
                        start: comment_start,
                        end: start,
                        kind: TokenKind::Comment,
                    });
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                    tag_state = TagState::TagPI(peek_pos());
                }
            }
            TagState::Doctype(in_bracket) => {
                if ch == '[' && !in_bracket {
                    tag_state = TagState::Doctype(true);
                } else if ch == ']' && in_bracket {
                    tag_state = TagState::Doctype(false);
                } else if ch == '>' && !in_bracket {
                    tag_state = TagState::TagNone(start);
                } else if ch == '<' {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Punctuation,
                    });
                } else if ch == '\n' {
                    tokens.push(Token {
                        start,
                        end: peek_pos(),
                        kind: TokenKind::Newline,
                    });
                }
            }
        }
    }

    match tag_state {
        TagState::TagNone(plain_start) => {
            tokens.push(Token {
                start: plain_start,
                end: string.len(),
                kind: TokenKind::PlainText,
            });
        }
        _ => {}
    }

    fn peek(tag_stack: &[&str], index: usize, check: &str) -> bool {
        if tag_stack.len() <= index {
            return false;
        }
        tag_stack.get(tag_stack.len() - index - 1) == Some(&check)
    }

    let mut result = Vec::new();
    let mut script = String::new();
    let mut style = String::new();
    let mut tag_stack = Vec::new();
    let mut in_script = false;
    let mut script_start = 0;
    let mut in_style = false;
    let mut style_start = 0;
    let mut literal_is_id = false;
    let mut literal_is_js = false;
    let mut literal_is_css = false;
    for token in tokens {
        match token.kind {
            TokenKind::TagName | TokenKind::EndTagName => {
                let tag_name = &string[token.start..token.end];
                if token.kind == TokenKind::TagName {
                    tag_stack.push(tag_name);
                } else {
                    tag_stack.pop();
                }
                result.push(token);
            }
            TokenKind::TagAttrName => {
                let attr_name = &string[token.start..token.end];
                if (peek(&tag_stack, 0, "field")
                    || peek(&tag_stack, 0, "property")
                    || peek(&tag_stack, 0, "method")
                    || peek(&tag_stack, 0, "parameter"))
                    && attr_name == "name"
                {
                    literal_is_id = true;
                }

                if attr_name.starts_with("on") {
                    literal_is_js = true;
                }
                if attr_name == "style" {
                    literal_is_css = true;
                }

                result.push(token);
            }
            TokenKind::PlainText | TokenKind::Newline | TokenKind::Comment => {
                let text = &string[token.start..token.end];
                if in_script {
                    script.push_str(text);
                } else if in_style {
                    style.push_str(text);
                } else {
                    result.push(token);
                }
            }
            TokenKind::StringLiteral => {
                if literal_is_id {
                    literal_is_id = false;
                    result.push(Token {
                        start: token.start,
                        end: token.end,
                        kind: TokenKind::Identifier(None),
                    });
                } else if literal_is_js {
                    literal_is_js = false;

                    let script_start = token.start;
                    let script = &string[token.start..token.end];
                    let script_toks = tokenize_c_like(&script.to_owned(), script_spec);
                    let script_toks = script_toks.into_iter().map(|t| Token {
                        start: t.start + script_start,
                        end: t.end + script_start,
                        kind: t.kind,
                    });
                    result.extend(script_toks);
                } else if literal_is_css {
                    literal_is_css = false;

                    let css_start = token.start;
                    let css = &string[token.start..token.end];
                    let css_toks = tokenize_css(&css);
                    let css_toks = css_toks.into_iter().map(|t| Token {
                        start: t.start + css_start,
                        end: t.end + css_start,
                        kind: t.kind,
                    });
                    result.extend(css_toks);
                } else {
                    result.push(token);
                }
            }
            TokenKind::Punctuation => {
                let punc = &string[token.start..token.end];

                if punc == "/>" {
                    tag_stack.pop();
                }

                let starting_script = peek(&tag_stack, 0, "script")
                    || peek(&tag_stack, 0, "constructor")
                    || peek(&tag_stack, 0, "destructor")
                    || peek(&tag_stack, 0, "handler")
                    || peek(&tag_stack, 0, "field")
                    || (peek(&tag_stack, 1, "method") && peek(&tag_stack, 0, "body"))
                    || (peek(&tag_stack, 1, "property") && peek(&tag_stack, 0, "getter"))
                    || (peek(&tag_stack, 1, "property") && peek(&tag_stack, 0, "setter"))
                    || false;

                let starting_style = peek(&tag_stack, 0, "style");

                if starting_script && punc == ">" {
                    in_script = true;
                    script_start = token.end;
                    result.push(token);
                } else if in_script && punc == "</" {
                    let script_toks = tokenize_c_like(&script, script_spec);
                    let script_toks = script_toks.into_iter().map(|t| Token {
                        start: t.start + script_start,
                        end: t.end + script_start,
                        kind: t.kind,
                    });
                    result.extend(script_toks);

                    script = String::new();

                    in_script = false;
                    result.push(token);
                } else if in_script {
                    script.push_str(punc);
                } else if starting_style && punc == ">" {
                    in_style = true;
                    style_start = token.end;
                    result.push(token);
                } else if in_style && punc == "</" {
                    let style_toks = tokenize_css(&style);
                    let style_toks = style_toks.into_iter().map(|t| Token {
                        start: t.start + style_start,
                        end: t.end + style_start,
                        kind: t.kind,
                    });
                    result.extend(style_toks);

                    style = String::new();

                    in_style = false;
                    result.push(token);
                } else if in_style {
                    style.push_str(punc);
                } else {
                    result.push(token);
                }
            }
            _ => {
                assert!(!in_script);
                assert!(!in_style);
                result.push(token);
            }
        }
    }

    if in_script {
        let script_toks = tokenize_c_like(&script, script_spec);
        let script_toks = script_toks.into_iter().map(|t| Token {
            start: t.start + script_start,
            end: t.end + script_start,
            kind: t.kind,
        });
        result.extend(script_toks);
    }
    if in_style {
        let style_toks = tokenize_css(&style);
        let style_toks = style_toks.into_iter().map(|t| Token {
            start: t.start + style_start,
            end: t.end + style_start,
            kind: t.kind,
        });
        result.extend(style_toks);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::*;

    fn token_matches(start: usize, end: usize, kind: TokenKind, rhs: &Token) {
        assert_eq!(start, rhs.start, "start matches");
        assert_eq!(end, rhs.end, "end matches");
        assert_eq!(kind, rhs.kind, "kind matches");
    }

    fn check_tokens(s: &str, expected: &[(&str, TokenKind)], spec: &LanguageSpec) {
        let toks = tokenize_c_like(&s, spec);
        check_tokens_match(s, &toks, expected);
    }

    fn check_tokens_match(s: &str, toks: &[Token], expected: &[(&str, TokenKind)]) {
        assert_eq!(
            toks.len(),
            expected.len(),
            "should have the same number of tokens"
        );

        for (a, b) in toks.iter().zip(expected.iter()) {
            assert_eq!(&s[a.start..a.end], b.0, "token strings should match");
            assert_eq!(a.kind, b.1, "token types should match");
        }
    }

    fn check_css_tokens(s: &str, expected: &[(&str, TokenKind)]) {
        let toks = tokenize_css(s);
        println!("{:#?}", toks);
        check_tokens_match(s, &toks, expected);
    }

    #[test]
    fn test_raw_strings_cpp() {
        let spec = match select_formatting("test.cpp") {
            FormatAs::FormatCLike(spec) => spec,
            _ => panic!("wrong spec"),
        };

        // NB: Expects `R"...";` (note the trailing semicolon to ensure we don't
        // simply grab the whole string as a single token.
        let check_simple = |s: &str| {
            let simple_raw = String::from(s);
            token_matches(
                0,
                simple_raw.len() - 1,
                TokenKind::StringLiteral,
                tokenize_c_like(&simple_raw, spec).first().unwrap(),
            );
        };

        check_simple(r##"R"(hello)";"##);
        check_simple(r##"R"foo(hel"lo)foo";"##);
        check_simple(r##"R"foo(hello)fooo)foo";"##);
        check_simple(r##"R"124.foo(hello)fooo)124.foo";"##);
        check_simple(r##"R"1234567890123456(just long enough)1234567890123456";"##);

        let check_empty = |s: &str| {
            let simple_raw = String::from(s);
            assert!(tokenize_c_like(&simple_raw, spec).is_empty());
        };

        check_empty(r##"R"foo(unterminated string literal)""##);
        check_empty(r##"R"foo"##); // unterminated sentinel
    }

    #[test]
    fn test_template_strings_js() {
        let spec = match select_formatting("test.js") {
            FormatAs::FormatCLike(spec) => spec,
            _ => {
                panic!("wrong spec");
            }
        };

        let check = |s: &str, expected: &[(&str, TokenKind)]| {
            check_tokens(s, expected, spec);
        };

        check(
            r##"`Hello, world`"##,
            &vec![("`Hello, world`", TokenKind::StringLiteral)],
        );
        check(
            r##"`Hello ${'w' + 'orld'}`"##,
            &vec![
                ("`Hello ${", TokenKind::StringLiteral),
                ("'w'", TokenKind::StringLiteral),
                ("+", TokenKind::Punctuation),
                ("'orld'", TokenKind::StringLiteral),
                ("}`", TokenKind::StringLiteral),
            ],
        );
        check(
            r##"`Hello ${`${w}` + 'orld'}`"##,
            &vec![
                ("`Hello ${", TokenKind::StringLiteral),
                ("`${", TokenKind::StringLiteral),
                ("w", TokenKind::Identifier(None)),
                ("}`", TokenKind::StringLiteral),
                ("+", TokenKind::Punctuation),
                ("'orld'", TokenKind::StringLiteral),
                ("}`", TokenKind::StringLiteral),
            ],
        );
        check(
            r##"`${() => { 'no}op' } + 'oop'}`"##,
            &vec![
                ("`${", TokenKind::StringLiteral),
                ("(", TokenKind::Punctuation),
                (")", TokenKind::Punctuation),
                ("=", TokenKind::Punctuation),
                (">", TokenKind::Punctuation),
                ("{", TokenKind::Punctuation),
                ("'no}op'", TokenKind::StringLiteral),
                ("}", TokenKind::Punctuation),
                ("+", TokenKind::Punctuation),
                ("'oop'", TokenKind::StringLiteral),
                ("}`", TokenKind::StringLiteral),
            ],
        );
    }

    #[test]
    fn check_newlines() {
        let js_spec = match select_formatting("test.js") {
            FormatAs::FormatCLike(spec) => spec,
            _ => {
                panic!("wrong spec");
            }
        };
        let cpp_spec = match select_formatting("test.cpp") {
            FormatAs::FormatCLike(spec) => spec,
            _ => {
                panic!("wrong spec");
            }
        };

        // C++ raw literal with a newline:
        check_tokens(
            "R\"(foo\nbar)\"",
            &vec![
                ("R\"(foo", TokenKind::StringLiteral),
                ("\n", TokenKind::Newline),
                ("bar)\"", TokenKind::StringLiteral),
            ],
            &cpp_spec,
        );
        check_tokens(
            "/* one /* line\nanother line */",
            &vec![
                ("/* one /* line", TokenKind::Comment),
                ("\n", TokenKind::Newline),
                ("another line */", TokenKind::Comment),
            ],
            &cpp_spec,
        );
        check_tokens(
            "`Hello ${world\n}\nanother line`",
            &vec![
                ("`Hello ${", TokenKind::StringLiteral),
                ("world", TokenKind::Identifier(None)),
                ("\n", TokenKind::Newline),
                ("}", TokenKind::StringLiteral),
                ("\n", TokenKind::Newline),
                ("another line`", TokenKind::StringLiteral),
            ],
            &js_spec,
        );
    }

    #[test]
    fn check_rust_stuff() {
        let rust_spec = match select_formatting("test.rs") {
            FormatAs::FormatCLike(spec) => spec,
            _ => {
                panic!("wrong spec");
            }
        };

        // Rust byte strings
        check_tokens(
            r##"b'a' b"bbb""##,
            &vec![
                ("b'a'", TokenKind::StringLiteral),
                (r#"b"bbb""#, TokenKind::StringLiteral),
            ],
            &rust_spec,
        );

        // Rust labels
        check_tokens(
            "&'static",
            &vec![
                ("&", TokenKind::Punctuation),
                ("'", TokenKind::Punctuation),
                ("static", TokenKind::Identifier(None)),
            ],
            &rust_spec,
        );
        check_tokens(
            "&'static ",
            &vec![
                ("&", TokenKind::Punctuation),
                ("'", TokenKind::Punctuation),
                ("static", TokenKind::Identifier(None)),
            ],
            &rust_spec,
        );
        check_tokens(
            "'label: while",
            &vec![
                ("'", TokenKind::Punctuation),
                ("label", TokenKind::Identifier(None)),
                (":", TokenKind::Punctuation),
                (
                    "while",
                    TokenKind::Identifier(Some(String::from("class=\"syn_reserved\" "))),
                ),
            ],
            &rust_spec,
        );
        check_tokens(
            "'\\n' while",
            &vec![
                ("'\\n'", TokenKind::StringLiteral),
                (
                    "while",
                    TokenKind::Identifier(Some(String::from("class=\"syn_reserved\" "))),
                ),
            ],
            &rust_spec,
        );
        check_tokens(
            "'b' while",
            &vec![
                ("'b'", TokenKind::StringLiteral),
                (
                    "while",
                    TokenKind::Identifier(Some(String::from("class=\"syn_reserved\" "))),
                ),
            ],
            &rust_spec,
        );

        // Rust raw strings
        check_tokens(
            r##"r#"hello"world"#"##,
            &vec![(r##"r#"hello"world"#"##, TokenKind::StringLiteral)],
            &rust_spec,
        );
        check_tokens(
            r#"r"hello world""#,
            &vec![(r#"r"hello world""#, TokenKind::StringLiteral)],
            &rust_spec,
        );
        check_tokens(
            r###"r##"hello world"# there"##"###,
            &vec![(
                r###"r##"hello world"# there"##"###,
                TokenKind::StringLiteral,
            )],
            &rust_spec,
        );
        check_tokens(
            "br\"hello world\"",
            &vec![("br\"hello world\"", TokenKind::StringLiteral)],
            &rust_spec,
        );
        check_tokens(
            "br#\"hello world \" there\"#",
            &vec![("br#\"hello world \" there\"#", TokenKind::StringLiteral)],
            &rust_spec,
        );

        // Rust nested comments
        check_tokens(
            "/* hello world */",
            &vec![("/* hello world */", TokenKind::Comment)],
            &rust_spec,
        );
        check_tokens(
            "/* hello /* world */ there */",
            &vec![("/* hello /* world */ there */", TokenKind::Comment)],
            &rust_spec,
        );
        check_tokens(
            "/*/**/*/",
            &vec![("/*/**/*/", TokenKind::Comment)],
            &rust_spec,
        );

        // Rust numbers
        // NB: This result is a little unexpected, but it's fine since we
        // don't actually need to generate code. The resulting output should
        // look the same as though we actually parsed `1.5`.
        check_tokens(
            "1.5",
            &vec![
                ("1", TokenKind::Identifier(None)),
                (".", TokenKind::Punctuation),
                ("5", TokenKind::Identifier(None)),
            ],
            &rust_spec,
        );
    }

    #[test]
    fn test_preproc_cpp() {
        let cpp_spec = match select_formatting("test.cpp") {
            FormatAs::FormatCLike(spec) => spec,
            _ => panic!("wrong spec"),
        };

        check_tokens(
            "#define",
            &vec![(
                "#define",
                TokenKind::Identifier(Some("class=\"syn_reserved\" ".to_string())),
            )],
            &cpp_spec,
        );

        check_tokens(
            "#  \t  \t  define",
            &vec![(
                "#  \t  \t  define",
                TokenKind::Identifier(Some("class=\"syn_reserved\" ".to_string())),
            )],
            &cpp_spec,
        );
    }

    #[test]
    fn test_css() {
        check_css_tokens(
            ".foo { bar: baz}\n#bar{}",
            &[
                (".", TokenKind::Punctuation),
                ("foo", TokenKind::Identifier(None)),
                (" ", TokenKind::PlainText),
                ("{", TokenKind::Punctuation),
                (" ", TokenKind::PlainText),
                (
                    "bar",
                    TokenKind::Identifier(Some(crate::languages::SYN_RESERVED_CLASS.into())),
                ),
                (":", TokenKind::Punctuation),
                (" ", TokenKind::PlainText),
                ("baz", TokenKind::Identifier(None)),
                ("}", TokenKind::Punctuation),
                ("\n", TokenKind::Newline),
                ("#bar", TokenKind::Identifier(None)),
                ("{", TokenKind::Punctuation),
                ("}", TokenKind::Punctuation),
            ],
        );
    }
}
