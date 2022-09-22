use globset::{GlobBuilder, GlobMatcher};

/// Parses `.gitignore` and `.eslintignore` style files so that a boolean test
/// can be run against a given file and see if it matches the file.  In most
/// cases it's probably more wise to instead use the output of whatever tool
/// would normally process this file rather than trying to replicate what it is
/// doing, but this is intended to help people get to a prototype or 80%
/// solution quickly.
///
/// We understand the file format to consist of:
/// - Comment lines starting with `#` which are ignored.
/// - Whitespace lines (after trimming/stripping) which are ignored.
/// - Negation lines which start with `!` and which are followed by a glob.
/// - Escaped lines start with a backslash to allow paths that start with the
///   above characters; the backslash will be removed.
/// - Everything else should be a glob.
///
/// In terms of glob semantics:
/// - We use the `globset` crate for this because we already use it.  It's
///   actually more capable than some of these formats require, but we don't
///   really care.
/// - We strip leading and trailing slashes on the glob patterns before handing
///   them to globset because mozsearch paths currently will not contain those
///   and we are not going to distinguish between files and directories at this
///   time in the way .gitignore will.
/// - We currently only evaluate on the full paths, effectively anchoring the
///   globs to the root, regardless of where the file came from.  This could be
///   enhanced for fidelity (and maybe a different mode introduced?) if someone
///   really cares.
///   - Additionally to this, we don't bother approximating the `.gitignore`
///     semantics where a glob applies at any level of the path unless there is
///     a leading `/` or a `/` in the middle of a path.  (A `/` at the end of
///     the glob is to distinguish directories from files.)   We could perhaps
///     approximate this by prepending a `**` if there's no leading/middle `/`
///     but as per the above, we don't need the fidelity right now, but aren't
///     opposed to adding it.  (Although I'm not sure we want to add a crate
///     dep for it.)
///
/// The general implementation for evaluating a filename:
/// - We maintain a list of globs, each of which has a negation state.  This is
///   populated when the struct is instantiated.
/// - We maintain a "matched" state which is initially false for each eval.
/// - If we match a negated glob, we clear the "matched" state.
/// - If we match a non-negated glob, we set the "matched" state.
/// - We return the "matched" state at the end.
pub struct GlobbingFileList {
    globs: Vec<(bool, GlobMatcher)>,
}

impl GlobbingFileList {
    pub fn new(file_contents: String) -> Self {
        let mut globs = vec![];

        for mut line in file_contents.lines() {
            line = line.trim();
            let mut negated = false;

            if line.is_empty() || line.starts_with("#") {
                continue;
            } else if line.starts_with("\\") {
                line = &line[1..];
            } else if line.starts_with("!") {
                negated = true;
                line = &line[1..];
            }

            // If the line ends with a "/" then add a ** glob on the end to make
            // it match all of its children too.
            let use_line = if line.ends_with("/") {
                format!("{}**", line)
            } else {
                line.to_string()
            };

            info!("  glob: '{}' normalized to '{}' negated: {}", line, &use_line, negated);
            if let Ok(glob) = GlobBuilder::new(&use_line)
                .literal_separator(true)
                .build() {
                globs.push((negated, glob.compile_matcher()));
            }
        }

        GlobbingFileList {
            globs
        }
    }

    pub fn is_match(&self, path: &str) -> bool {
        let mut matches = false;

        for (negated, matcher) in &self.globs {
            if matcher.is_match(path) {
                // (I feel this reads better than assigning `!*negated`.)
                if *negated {
                    matches = false;
                } else {
                    matches = true;
                }
            }
        }

        matches
    }
}
