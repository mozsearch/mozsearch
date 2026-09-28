//! Detection of license headers and editor modelines ("boilerplate") in
//! comments and plain text, so that their tokens can be classified as
//! `TokenClass::Boilerplate` rather than as ordinary comments.
//!
//! Boilerplate is shared by huge numbers of files (the MPL 2.0 header is in
//! over 50,000 Firefox files), so it shouldn't count as evidence that two files
//! are related (ex: when deciding whether git's similarity-based rename
//! detection paired a file with an unrelated file) and it isn't interesting to
//! track in per-token histories.  Other comments, including large block
//! comments at the top of files, are not boilerplate.
//!
//! Detection works on the word sequence rather than the comment structure: we
//! ignore tokens without alphanumeric characters (comment markers like `/*`,
//! `*`, `#`, `//`), compare words case-insensitively with surrounding
//! punctuation removed, and treat "https://" and "http://" the same.  This
//! means we don't care whether a header is one block comment or several line
//! comments or how its lines wrap.  We recognize:
//! - Phrases: the MPL 2.0 header and its "Incompatible With Secondary Licenses"
//!   exhibit, the public domain dedication used by tests, and the sentences of
//!   the license texts most common in third-party code in Firefox (test262,
//!   Chromium, WebRTC, LLVM, Apache 2.0, BSD, MIT, GNU, Unicode, AOMedia,
//!   FreeType).  Phrases are sentences rather than whole license texts so that
//!   variants still mostly match.
//! - The MPL 1.1 era tri-license blocks, from `BEGIN LICENSE BLOCK` to
//!   `END LICENSE BLOCK`.
//! - Lines: `SPDX-License-Identifier:` lines, copyright notice lines, emacs
//!   `-*- ... -*-` modelines, vim/vi/ex modelines, and `@license` tags.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::file_format::history::syntax_files::{TokenClass, format_token_line};

/// A token produced by one of our tokenizers before formatting.  `text` should
/// be a slice of the source being tokenized; see `finish_tokens`.
pub struct RawToken<'a> {
    pub context: String,
    pub class: TokenClass,
    pub text: &'a str,
}

/// Tokens formatted as `history/syntax/files` lines by `finish_tokens`.
pub struct FinishedTokens {
    pub lines: Vec<String>,
    /// The byte offset of each token in the source, or None if the token isn't
    /// a slice of the source.  (All of our tokenizers currently produce slices,
    /// but this was not always the case.)  The token's length is the length of
    /// the token in its line.
    pub offsets: Vec<Option<u32>>,
}

/// Mark boilerplate in runs of consecutive comment/text tokens and format the
/// tokens as `history/syntax/files` lines.  Line starts and offsets are
/// recovered from the tokens' positions in `source`.  Tokens which aren't
/// slices of `source` are never line starts and don't affect whether the next
/// token is.
pub fn finish_tokens(source: &str, mut tokens: Vec<RawToken>) -> FinishedTokens {
    let base = source.as_ptr() as usize;
    let mut line_starts = Vec::with_capacity(tokens.len());
    let mut offsets = Vec::with_capacity(tokens.len());
    let mut prev_end: Option<usize> = None;
    for token in &tokens {
        let offset = (token.text.as_ptr() as usize)
            .checked_sub(base)
            .filter(|offset| offset + token.text.len() <= source.len());
        offsets.push(offset.and_then(|offset| u32::try_from(offset).ok()));
        let line_start = match (offset, prev_end) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(start), Some(prev_end)) => {
                prev_end <= start && source[prev_end..start].contains('\n')
            }
        };
        line_starts.push(line_start);
        if let Some(start) = offset {
            prev_end = Some(start + token.text.len());
        }
    }

    let is_prose = |class: TokenClass| matches!(class, TokenClass::Comment | TokenClass::Text);
    let mut i = 0;
    while i < tokens.len() {
        if !is_prose(tokens[i].class) {
            i += 1;
            continue;
        }
        let start = i;
        while i < tokens.len() && is_prose(tokens[i].class) {
            i += 1;
        }
        let run: Vec<BoilerplateToken> = (start..i)
            .map(|j| BoilerplateToken {
                text: tokens[j].text,
                line_start: line_starts[j],
            })
            .collect();
        for (j, marked) in find_boilerplate(&run).into_iter().enumerate() {
            if marked {
                tokens[start + j].class = TokenClass::Boilerplate;
            }
        }
    }

    FinishedTokens {
        lines: tokens
            .iter()
            .map(|t| format_token_line(&t.context, t.class, t.text))
            .collect(),
        offsets,
    }
}

/// A token for boilerplate detection: its text and whether it is the first
/// token on its source line.
#[derive(Clone, Copy, Debug)]
pub struct BoilerplateToken<'a> {
    pub text: &'a str,
    pub line_start: bool,
}

/// Boilerplate phrases.  A `*` matches up to `MAX_WILDCARD_WORDS` words (ex: the
/// copyright holder's name).
const PHRASES: &[&str] = &[
    // ## Mozilla
    "This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. \
     If a copy of the MPL was not distributed with this file, You can obtain one at \
     http://mozilla.org/MPL/2.0/.",
    "This Source Code Form is \"Incompatible With Secondary Licenses\", as defined by the \
     Mozilla Public License, v. 2.0.",
    "Any copyright is dedicated to the Public Domain. \
     http://creativecommons.org/publicdomain/zero/1.0/",
    "Any copyright is dedicated to the Public Domain. \
     http://creativecommons.org/licenses/publicdomain/",
    // ## test262
    "This code is governed by the BSD license found in the LICENSE file.",
    "This code is governed by the license found in the LICENSE file.",
    // ## Chromium and WebRTC
    "Use of this source code is governed by a BSD-style license that can be found in the \
     LICENSE file.",
    "Use of this source code is governed by a BSD-style license that can be found in the \
     LICENSE file in the root of the source tree.",
    "An additional intellectual property rights grant can be found in the file PATENTS.",
    "All contributing project authors may be found in the AUTHORS file in the root of the \
     source tree.",
    // ## LLVM
    "Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions. \
     See https://llvm.org/LICENSE.txt for license information.",
    // ## Apache 2.0
    "Licensed under the Apache License, Version 2.0 (the \"License\"); you may not use this \
     file except in compliance with the License.",
    "You may obtain a copy of the License at http://www.apache.org/licenses/LICENSE-2.0",
    "Unless required by applicable law or agreed to in writing, software distributed under \
     the License is distributed on an \"AS IS\" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF \
     ANY KIND, either express or implied.",
    "See the License for the specific language governing permissions and limitations under \
     the License.",
    // ## BSD
    "Redistribution and use in source and binary forms, with or without modification, are \
     permitted provided that the following conditions are met:",
    "Redistributions of source code must retain the above copyright notice, this list of \
     conditions and the following disclaimer.",
    "Redistributions in binary form must reproduce the above copyright notice, this list of \
     conditions and the following disclaimer in the documentation and/or other materials \
     provided with the distribution.",
    "Neither the name of * nor the names of its contributors may be used to endorse or \
     promote products derived from this software without specific prior written permission.",
    "THIS SOFTWARE IS PROVIDED BY * \"AS IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, \
     INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR \
     A PARTICULAR PURPOSE ARE DISCLAIMED.",
    "IN NO EVENT SHALL * BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, \
     OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS \
     OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND \
     ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING \
     NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF \
     ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.",
    // ## MIT
    "Permission is hereby granted, free of charge, to any person obtaining a copy of this \
     software and associated documentation files (the \"Software\"), to deal in the Software \
     without restriction, including without limitation the rights to use, copy, modify, \
     merge, publish, distribute, sublicense, and/or sell copies of the Software, and to \
     permit persons to whom the Software is furnished to do so, subject to the following \
     conditions:",
    "The above copyright notice and this permission notice shall be included in all copies \
     or substantial portions of the Software.",
    "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, \
     INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A \
     PARTICULAR PURPOSE AND NONINFRINGEMENT.",
    "IN NO EVENT SHALL * BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN \
     ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE \
     SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.",
    // ## GNU (L)GPL, which is preceded by the program name.
    "is free software; you can redistribute it and/or modify it under the terms of the GNU * \
     General Public License as published by the Free Software Foundation; either version * of \
     the License, or (at your option) any later version.",
    "is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even \
     the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU * \
     General Public License for more details.",
    "You should have received a copy of the GNU * General Public License along with * if not, \
     write to the Free Software Foundation, Inc.,",
    "You should have received a copy of the GNU * General Public License along with * If not, \
     see <http://www.gnu.org/licenses/>.",
    "51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA",
    "59 Temple Place, Suite 330, Boston, MA 02111-1307 USA",
    // ## Unicode (ICU), after a "© 2016 and later: Unicode, Inc. and others." line.
    "License & terms of use: http://www.unicode.org/copyright.html",
    // ## AOMedia
    "This source code is subject to the terms of the BSD 2 Clause License and the Alliance for \
     Open Media Patent License 1.0.",
    "If the BSD 2 Clause License was not distributed with this source code in the LICENSE file, \
     you can obtain it at www.aomedia.org/license/software.",
    "If the Alliance for Open Media Patent License 1.0 was not distributed with this source code \
     in the PATENTS file, you can obtain it at www.aomedia.org/license/patent.",
    // ## FreeType
    "This file is part of the FreeType project, and may only be used, modified, and distributed \
     under the terms of the FreeType project license, LICENSE.TXT. By continuing to use, \
     modify, or distribute this file you indicate that you have read the license and \
     understand and accept it fully.",
];

const MAX_WILDCARD_WORDS: usize = 8;

/// The normalized words of each phrase (None for a wildcard), indexed by their
/// first word, which is never a wildcard.
static PHRASES_BY_FIRST_WORD: LazyLock<HashMap<String, Vec<Vec<Option<String>>>>> =
    LazyLock::new(|| {
        let mut map: HashMap<String, Vec<Vec<Option<String>>>> = HashMap::new();
        for phrase in PHRASES {
            let words: Vec<Option<String>> = phrase
                .split_whitespace()
                .filter_map(|w| match w {
                    "*" => Some(None),
                    _ => {
                        assert!(!w.contains('*'), "wildcards must be separate words");
                        normalize(w).map(Some)
                    }
                })
                .collect();
            let first = words[0]
                .clone()
                .expect("phrases can't start with a wildcard");
            map.entry(first).or_default().push(words);
        }
        map
    });

/// If `phrase` matches `words` starting at `start`, return the index of the
/// last matched word.
fn match_phrase(
    phrase: &[Option<String>],
    words: &[(usize, String)],
    start: usize,
) -> Option<usize> {
    let Some((first, rest)) = phrase.split_first() else {
        return start.checked_sub(1);
    };
    match first {
        Some(word) => {
            if words.get(start).is_some_and(|(_, w)| w == word) {
                match_phrase(rest, words, start + 1)
            } else {
                None
            }
        }
        None => (0..=MAX_WILDCARD_WORDS)
            .take_while(|n| start + n <= words.len())
            .find_map(|n| match_phrase(rest, words, start + n)),
    }
}

/// Normalize a word for comparison, returning None for tokens without any
/// alphanumeric characters (ex: comment markers).
fn normalize(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric());
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    Some(match lower.strip_prefix("https://") {
        Some(rest) => format!("http://{}", rest),
        None => lower,
    })
}

fn has_alphanumeric(token: &str) -> bool {
    token.chars().any(|c| c.is_alphanumeric())
}

fn is_year(word: &str) -> bool {
    word.len() == 4
        && (word.starts_with("19") || word.starts_with("20"))
        && word.chars().all(|c| c.is_ascii_digit())
}

/// Return which of the tokens (typically a run of consecutive comment or text
/// tokens) are boilerplate.
pub fn find_boilerplate(tokens: &[BoilerplateToken]) -> Vec<bool> {
    let mut marked = vec![false; tokens.len()];

    // The normalized words and the index of the token each came from.
    let words: Vec<(usize, String)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| normalize(t.text).map(|w| (i, w)))
        .collect();
    let mark_range = |marked: &mut Vec<bool>, first: usize, last: usize| {
        for m in &mut marked[first..=last] {
            *m = true;
        }
    };

    // ## Phrases
    for (start, (first_token, word)) in words.iter().enumerate() {
        for phrase in PHRASES_BY_FIRST_WORD.get(word).into_iter().flatten() {
            if let Some(end) = match_phrase(phrase, &words, start) {
                mark_range(&mut marked, *first_token, words[end].0);
            }
        }
    }

    // ## Tri-license blocks
    let find_seq = |from: usize, seq: &[&str]| -> Option<usize> {
        (from..words.len().saturating_sub(seq.len() - 1))
            .find(|&i| seq.iter().enumerate().all(|(j, s)| words[i + j].1 == *s))
    };
    let mut from = 0;
    while let Some(begin) = find_seq(from, &["begin", "license", "block"]) {
        let Some(end) = find_seq(begin + 3, &["end", "license", "block"]) else {
            break;
        };
        mark_range(&mut marked, words[begin].0, words[end + 2].0);
        from = end + 3;
    }

    // ## Line-based rules
    let mut line_start_idx = 0;
    for i in 0..=tokens.len() {
        if i < tokens.len() && (i == 0 || !tokens[i].line_start) {
            continue;
        }
        // tokens[line_start_idx..i] is a line.
        let line = line_start_idx..i;
        line_start_idx = i;
        if line.is_empty() {
            continue;
        }
        let line_words: Vec<(usize, String)> = line
            .clone()
            .filter_map(|j| normalize(tokens[j].text).map(|w| (j, w)))
            .collect();
        let Some((first_word_idx, first_word)) = line_words.first() else {
            continue;
        };
        // The first token that isn't just a comment marker, which is where a
        // `© 2016 ...` notice starts since "©" isn't alphanumeric.
        let first_text = line
            .clone()
            .find(|&j| has_alphanumeric(tokens[j].text) || tokens[j].text.contains('©'));

        // SPDX-License-Identifier through the end of the line.
        if let Some((j, _)) = line_words
            .iter()
            .find(|(_, w)| w.starts_with("spdx-license-identifier"))
        {
            mark_range(&mut marked, *j, line.end - 1);
        }

        // Copyright notices.
        let starts_with_symbol = first_text.is_some_and(|j| tokens[j].text.starts_with('©'));
        let is_notice_start = first_word == "copyright" || first_word == "c" || starts_with_symbol;
        let has_year_or_symbol = line_words
            .iter()
            .any(|(_, w)| is_year(w) || w.contains('©'))
            || line
                .clone()
                .any(|j| tokens[j].text == "(c)" || tokens[j].text == "©");
        if is_notice_start && has_year_or_symbol {
            let first = match first_text {
                Some(j) if starts_with_symbol => j,
                _ => *first_word_idx,
            };
            mark_range(&mut marked, first, line.end - 1);
        }

        // vim/vi/ex modelines through the end of the line.
        if let Some(j) = line.clone().find(|&j| {
            let t = tokens[j].text;
            ["vim:", "vi:", "ex:"]
                .iter()
                .any(|prefix| t.starts_with(prefix))
        }) {
            mark_range(&mut marked, j, line.end - 1);
        }

        // emacs modelines from the first to the last `-*-`, which may be
        // embedded in other tokens (ex: LLVM's `---*- C++ -*-===//` banners).
        let emacs: Vec<usize> = line
            .clone()
            .filter(|&j| tokens[j].text.contains("-*-"))
            .collect();
        if let (Some(&first), Some(&last)) = (emacs.first(), emacs.last())
            && (first != last || tokens[first].text.matches("-*-").count() >= 2)
        {
            mark_range(&mut marked, first, last);
        }

        // `@license` tags.
        for j in line.clone() {
            if tokens[j].text == "@license" {
                marked[j] = true;
            }
        }
    }

    // ## Extend over adjacent comment markers on the same lines.
    //
    // This picks up things like the `/*` and `*/` around a license comment
    // without extending into a following comment on a later line.
    for i in 1..tokens.len() {
        if marked[i - 1] && !marked[i] && !has_alphanumeric(tokens[i].text) && !tokens[i].line_start
        {
            marked[i] = true;
        }
    }
    for i in (0..tokens.len().saturating_sub(1)).rev() {
        if marked[i + 1]
            && !marked[i]
            && !has_alphanumeric(tokens[i].text)
            && !tokens[i + 1].line_start
        {
            marked[i] = true;
        }
    }

    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Split source text into boilerplate tokens by whitespace, noting line
    /// starts, and return the marked words joined by spaces plus the unmarked
    /// words joined by spaces.
    fn check(source: &str) -> (String, String) {
        let mut tokens = vec![];
        for line in source.lines() {
            for (i, word) in line.split_whitespace().enumerate() {
                tokens.push(BoilerplateToken {
                    text: word,
                    line_start: i == 0,
                });
            }
        }
        let marked = find_boilerplate(&tokens);
        let pick = |want: bool| {
            tokens
                .iter()
                .zip(&marked)
                .filter(|(_, m)| **m == want)
                .map(|(t, _)| t.text)
                .collect::<Vec<_>>()
                .join(" ")
        };
        (pick(true), pick(false))
    }

    #[test]
    fn test_mpl_header_with_following_block_comment() {
        let (marked, rest) = check(
            "/* This Source Code Form is subject to the terms of the Mozilla Public\n\
              * License, v. 2.0. If a copy of the MPL was not distributed with this\n\
              * file, You can obtain one at http://mozilla.org/MPL/2.0/. */\n\
             /*\n\
              * This is a large design comment which is definitely not boilerplate\n\
              * and whose history must be preserved when the file is renamed.\n\
              */",
        );
        assert!(marked.starts_with("/* This Source"), "{}", marked);
        assert!(
            marked.ends_with("http://mozilla.org/MPL/2.0/. */"),
            "{}",
            marked
        );
        assert_eq!(
            rest,
            "/* * This is a large design comment which is definitely not boilerplate \
             * and whose history must be preserved when the file is renamed. */"
        );
    }

    #[test]
    fn test_python_and_https_variants() {
        let (marked, rest) = check(
            "# This Source Code Form is subject to the terms of the Mozilla Public\n\
             # License, v. 2.0. If a copy of the MPL was not distributed with this\n\
             # file, You can obtain one at https://mozilla.org/MPL/2.0/.\n\
             # Real comment.",
        );
        assert!(
            marked.ends_with("https://mozilla.org/MPL/2.0/."),
            "{}",
            marked
        );
        assert_eq!(rest, "# Real comment.");
    }

    #[test]
    fn test_public_domain() {
        let (_, rest) = check(
            "/* Any copyright is dedicated to the Public Domain.\n\
              * http://creativecommons.org/publicdomain/zero/1.0/ */\n\
             // Test that things work.",
        );
        assert_eq!(rest, "// Test that things work.");
    }

    #[test]
    fn test_license_block_and_modelines() {
        let (marked, rest) = check(
            "/* -*- Mode: C++; tab-width: 2; indent-tabs-mode: nil; c-basic-offset: 2 -*- */\n\
             /* vim: set ts=2 et sw=2 tw=80: */\n\
             /* ***** BEGIN LICENSE BLOCK *****\n\
              * Copyright 1993 by OpenVision Technologies, Inc.\n\
              * Permission to use, copy, modify...\n\
              ****** END LICENSE BLOCK ***** */\n\
             /* Keep this. */",
        );
        assert!(marked.contains("Mode: C++;"), "{}", marked);
        assert!(marked.contains("vim: set ts=2"), "{}", marked);
        assert!(marked.contains("Permission to use"), "{}", marked);
        assert_eq!(rest, "/* Keep this. */");
    }

    #[test]
    fn test_llvm_spdx_copyright() {
        let (_, rest) = check(
            "//===--- HeuristicResolver.cpp ---------------------------*- C++-*-===//\n\
             //\n\
             // Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.\n\
             // See https://llvm.org/LICENSE.txt for license information.\n\
             // SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception\n\
             /**\n\
              * @license\n\
              * Copyright 2017 Google Inc.\n\
              * SPDX-License-Identifier: Apache-2.0\n\
              */",
        );
        // Only the banner, the empty comment line, and the JSDoc delimiters
        // remain; markers on lines within the boilerplate are boilerplate too.
        assert_eq!(rest, "//===--- HeuristicResolver.cpp // /** */");
    }

    #[test]
    fn test_third_party_licenses() {
        let (_, rest) = check(
            "// Copyright (C) 2017 Mozilla Corporation. All rights reserved.\n\
             // This code is governed by the BSD license found in the LICENSE file.\n\
             /*---\n\
             description: Keep this.\n\
             ---*/",
        );
        assert_eq!(rest, "/*--- description: Keep this. ---*/");

        let (_, rest) = check(
            "/*\n\
              *  Copyright (c) 2012 The WebRTC project authors. All Rights Reserved.\n\
              *\n\
              *  Use of this source code is governed by a BSD-style license\n\
              *  that can be found in the LICENSE file in the root of the source\n\
              *  tree. An additional intellectual property rights grant can be found\n\
              *  in the file PATENTS.  All contributing project authors may\n\
              *  be found in the AUTHORS file in the root of the source tree.\n\
              */\n\
             // Keep this.",
        );
        assert_eq!(rest, "/* * */ // Keep this.");

        let (_, rest) = check(
            "/**\n\
              * Copyright 2017 Google Inc. All rights reserved.\n\
              *\n\
              * Licensed under the Apache License, Version 2.0 (the \"License\");\n\
              * you may not use this file except in compliance with the License.\n\
              * You may obtain a copy of the License at\n\
              *\n\
              *     https://www.apache.org/licenses/LICENSE-2.0\n\
              *\n\
              * Unless required by applicable law or agreed to in writing, software\n\
              * distributed under the License is distributed on an \"AS IS\" BASIS,\n\
              * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.\n\
              * See the License for the specific language governing permissions and\n\
              * limitations under the License.\n\
              */",
        );
        // The blank line within "You may obtain ... LICENSE-2.0" is marked too.
        assert_eq!(rest, "/** * * */");

        let (marked, rest) = check(
            "# Neither the name of Google Inc. nor the names of its\n\
             # contributors may be used to endorse or promote products derived from\n\
             # this software without specific prior written permission.\n\
             #\n\
             # THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS\n\
             # \"AS IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT\n\
             # LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR\n\
             # A PARTICULAR PURPOSE ARE DISCLAIMED.\n\
             # Neither the name of a project with a very long name that is far too long nor the \
             names of its contributors may be used to endorse or promote products derived from \
             this software without specific prior written permission.",
        );
        assert!(marked.contains("Google Inc."), "{}", marked);
        assert!(marked.contains("COPYRIGHT HOLDERS"), "{}", marked);
        // The wildcard doesn't match arbitrarily long runs of words.
        assert!(
            rest.starts_with("# # Neither the name of a project"),
            "{}",
            rest
        );
    }

    #[test]
    fn test_more_third_party_licenses() {
        let (_, rest) = check(
            "/*\n\
              * API for creating VLC trees\n\
              * Copyright (c) 2000, 2001 Fabrice Bellard\n\
              *\n\
              * This file is part of FFmpeg.\n\
              *\n\
              * FFmpeg is free software; you can redistribute it and/or\n\
              * modify it under the terms of the GNU Lesser General Public\n\
              * License as published by the Free Software Foundation; either\n\
              * version 2.1 of the License, or (at your option) any later version.\n\
              *\n\
              * FFmpeg is distributed in the hope that it will be useful,\n\
              * but WITHOUT ANY WARRANTY; without even the implied warranty of\n\
              * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU\n\
              * Lesser General Public License for more details.\n\
              *\n\
              * You should have received a copy of the GNU Lesser General Public\n\
              * License along with FFmpeg; if not, write to the Free Software\n\
              * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA\n\
              */",
        );
        // The program name isn't part of the phrases.
        assert_eq!(
            rest,
            "/* * API for creating VLC trees * * This file is part of FFmpeg. * * FFmpeg * * \
             FFmpeg * */"
        );

        let (_, rest) = check(
            "// © 2016 and later: Unicode, Inc. and others.\n\
             // License & terms of use: http://www.unicode.org/copyright.html\n\
             // Keep this.",
        );
        assert_eq!(rest, "// Keep this.");

        let (_, rest) = check(
            "/*\n\
              * Copyright (c) 2016, Alliance for Open Media. All rights reserved.\n\
              *\n\
              * This source code is subject to the terms of the BSD 2 Clause License and\n\
              * the Alliance for Open Media Patent License 1.0. If the BSD 2 Clause License\n\
              * was not distributed with this source code in the LICENSE file, you can\n\
              * obtain it at www.aomedia.org/license/software. If the Alliance for Open\n\
              * Media Patent License 1.0 was not distributed with this source code in the\n\
              * PATENTS file, you can obtain it at www.aomedia.org/license/patent.\n\
              */",
        );
        assert_eq!(rest, "/* * */");
    }

    #[test]
    fn test_not_boilerplate() {
        for source in [
            "// Copyright handling is complicated, see below.",
            "// This Source Code Form is lovely.",
            "/* The emacs -*- marker alone isn't a modeline. */",
            "// We use a license block to track BEGIN LICENSE BLOCK markers.",
        ] {
            let (marked, _) = check(source);
            assert_eq!(marked, "", "{}", source);
        }
    }
}
