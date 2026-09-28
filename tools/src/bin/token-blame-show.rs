// Debugging tool which prints the token-centric blame of a file per source
// line, as the blame strip presents it (see `tools::hyperblame::token_blame`).
//
// Usage:
//   token-blame-show [--tokens] HISTORY_DIR SOURCE_REPO PATH [SOURCE_REV]
//
// HISTORY_DIR is a tree's `history_path` (containing the "syntax" and
// "timeline" repos).  SOURCE_REV can be anything `git rev-parse` understands in
// SOURCE_REPO, and defaults to the revision the history's HEAD describes.
//
// Output has one line per source line:
//   LINENO REV[*] | SOURCE
// where "*" means the line's tokens were changed by several revisions (REV is
// the newest), and removals are shown on their own lines: "^^^" for a removal
// at the start of the file, "~~~" for a removal within the preceding line, and
// "___" for a removal after the preceding line.  With --tokens, each line is
// followed by its tokens and their history.  With --peephole TOKEN, it instead
// prints the peephole history (see `hyperblame::peephole`) of the token with
// that (1-based) index (or comma-separated indices, ex: a line's), and with
// --future TOKEN, where that token is now (see `hyperblame::future`).

extern crate git2;
extern crate tools;

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::exit;

use git2::{Oid, Repository};

use tools::file_format::history::timeline_annotated::{
    HyperLineData, HyperTokenRef, RemovalMarker,
};
use tools::hyperblame::future;
use tools::hyperblame::peephole::{Cursor, peephole_page};
use tools::hyperblame::token_blame::{TreeHistory, blame_tokens};

fn short(rev: &str) -> &str {
    &rev[..rev.len().min(8)]
}

fn describe_ref(r: &HyperTokenRef, path: &str) -> String {
    if r.path == path {
        format!("{}:{}", short(&r.source_rev), r.lineno)
    } else {
        format!("{}:{}:{}", short(&r.source_rev), r.path, r.lineno)
    }
}

fn describe_removal(marker: &RemovalMarker, path: &str) -> String {
    let mut s = format!(
        "{} removed {} tokens, first {}",
        short(&marker.source_rev),
        marker.num_removed,
        describe_ref(&marker.first_removed, path)
    );
    if marker.num_moved > 0 {
        s.push_str(&format!(" ({} moved)", marker.num_moved));
    }
    s
}

fn describe_token(data: &HyperLineData, path: &str) -> String {
    let mut s = describe_ref(&data.introduced, path);
    if let Some(predecessor) = &data.predecessor {
        s.push_str(&format!(" <= {}", describe_ref(predecessor, path)));
    }
    s
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let show_tokens = args.first().is_some_and(|arg| arg == "--tokens");
    if show_tokens {
        args.remove(0);
    }
    let mut peephole_tokens = None;
    if args.first().is_some_and(|arg| arg == "--peephole") && args.len() > 1 {
        peephole_tokens = Some(
            args[1]
                .split(',')
                .filter_map(|token| token.parse::<u32>().ok())
                .collect::<Vec<_>>(),
        );
        args.drain(..2);
    }
    let mut future_token = None;
    if args.first().is_some_and(|arg| arg == "--future") && args.len() > 1 {
        future_token = args[1].parse::<u32>().ok();
        args.drain(..2);
    }
    if !(3..=4).contains(&args.len()) {
        eprintln!(
            "Usage: token-blame-show [--tokens | --peephole TOKENS | --future TOKEN] HISTORY_DIR SOURCE_REPO PATH [SOURCE_REV]"
        );
        exit(1);
    }
    let history = TreeHistory::open(&args[0], None).unwrap_or_else(|e| {
        eprintln!("Couldn't open the history at {}: {}", args[0], e);
        exit(1);
    });
    let source_repo = Repository::open(&args[1]).unwrap();
    let path = &args[2];

    let timeline_commit = match args.get(3) {
        Some(rev) => {
            let source_rev = source_repo
                .revparse_single(rev)
                .and_then(|object| object.peel_to_commit())
                .unwrap_or_else(|e| {
                    eprintln!("Bad source revision {}: {}", rev, e);
                    exit(1);
                })
                .id();
            history.timeline_commit(source_rev).unwrap_or_else(|| {
                eprintln!("The history doesn't have source revision {}", source_rev);
                exit(1);
            })
        }
        None => history.head_timeline_commit().unwrap_or_else(|| {
            eprintln!("The history is empty");
            exit(1);
        }),
    };

    let file_history = match history.file_history(&timeline_commit, path) {
        Ok(Some(file_history)) => file_history,
        Ok(None) => {
            eprintln!("The history doesn't have {}", path);
            exit(1);
        }
        Err(e) => {
            eprintln!("Couldn't load the history of {}: {}", path, e);
            exit(1);
        }
    };
    if let Some(token) = future_token {
        match future::follow(&history, file_history.source_rev, path, token) {
            Ok(future) => println!("{}", serde_json::to_string_pretty(&future).unwrap()),
            Err(e) => eprintln!("Couldn't follow the token: {}", e),
        }
        return;
    }

    if let Some(tokens) = peephole_tokens {
        let start = Cursor {
            rev: file_history.source_rev.to_string(),
            path: path.clone(),
            tokens,
        };
        let page = peephole_page(&history, &source_repo, start, 50, usize::MAX);
        for step in &page.steps {
            let summary = Oid::from_str(&step.rev)
                .and_then(|oid| source_repo.find_commit(oid))
                .map(|commit| {
                    let message = commit.message().unwrap_or("");
                    message.lines().next().unwrap_or("").to_string()
                })
                .unwrap_or_default();
            println!("{} {}", short(&step.rev), summary);
            // Mark the tokens the commit introduced with brackets.
            let text: Vec<u16> = step.text.encode_utf16().collect();
            let mut shown = String::new();
            let mut pos = 0;
            for (i, (start, end, changed)) in step.tokens.iter().enumerate() {
                shown.push_str(&String::from_utf16_lossy(&text[pos..*start]));
                let token = String::from_utf16_lossy(&text[*start..*end]);
                let marked = match (*changed, step.anchors.contains(&i)) {
                    (true, true) => format!("[*{}]", token),
                    (true, false) => format!("[{}]", token),
                    (false, true) => format!("*{}", token),
                    (false, false) => token,
                };
                shown.push_str(&marked);
                pos = *end;
            }
            if step.removed > 0 {
                shown.push_str(&format!("  (-{} tokens)", step.removed));
            }
            println!("    {}", shown.replace('\n', "\n    "));
        }
        println!("end: {:?}, cost: {} MB", page.end, page.cost / 1_000_000);
        return;
    }

    let source_tree = source_repo
        .find_commit(file_history.source_rev)
        .and_then(|commit| commit.tree())
        .unwrap();
    let source_blob = source_tree
        .get_path(Path::new(path))
        .and_then(|entry| source_repo.find_blob(entry.id()))
        .unwrap();
    let source = std::str::from_utf8(source_blob.content()).unwrap();

    let blame = blame_tokens(source, &file_history).unwrap_or_else(|e| {
        eprintln!("Couldn't blame the tokens of {}: {}", path, e);
        exit(1);
    });
    let mut commit_times = HashMap::new();
    let lines = blame.lines(|rev| {
        *commit_times.entry(rev.to_string()).or_insert_with(|| {
            Oid::from_str(rev)
                .and_then(|oid| source_repo.find_commit(oid))
                .map_or(0, |commit| commit.time().seconds())
        })
    });

    println!(
        "source {} path {} ({})",
        file_history.source_rev, path, file_history.lang
    );
    if let Some(marker) = blame.removal_above() {
        println!("      ^^^ {}", describe_removal(marker, path));
    }
    for (i, line) in lines.iter().enumerate() {
        let start = blame.line_starts[i];
        let end = blame
            .line_starts
            .get(i + 1)
            .map_or(source.len(), |&next| next);
        let text = source[start..end].trim_end_matches('\n');
        println!(
            "{:>5} {:<8}{} | {}",
            i + 1,
            line.rev.map_or("", short),
            if line.mixed { "*" } else { " " },
            text
        );
        if show_tokens {
            for (index, token) in blame.tokens[line.tokens.clone()].iter().enumerate() {
                println!(
                    "      {:>5} {:<24} {}",
                    line.tokens.start + index + 1,
                    &source[token.range.clone()],
                    describe_token(&token.data, path)
                );
            }
        }
        for marker in &line.removals_within {
            println!("      ~~~ {}", describe_removal(marker, path));
        }
        if let Some(marker) = line.removal_below {
            println!("      ___ {}", describe_removal(marker, path));
        }
    }
}
