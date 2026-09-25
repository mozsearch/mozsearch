// Debugging tool which prints the token-centric blame for a file by combining
// the syntax repo's "files" representation of the file with the timeline repo's
// "annotated" representation.
//
// Usage:
//   hyperblame-show SYNTAX_REPO TIMELINE_REPO TIMELINE_REV PATH
//
// TIMELINE_REV can be anything `git rev-parse` understands in the timeline
// repo, like "HEAD" or "HEAD~3".
//
// Output has one line per token:
//   LINENO CONTEXT TOKEN | INTRODUCED [<= PREDECESSOR] [!! REMOVAL MARKER]
// where revisions are abbreviated and paths are omitted when they are "%".

extern crate git2;
extern crate tools;

use std::env;
use std::path::Path;

use git2::Repository;

use tools::file_format::config::timeline_commit_to_meta;
use tools::file_format::history::timeline_annotated::{HyperLineData, HyperTokenRef};
use tools::hyperblame::inference::{split_token_line, token_file_lines};

fn read_path(repo: &Repository, tree: &git2::Tree, path: &str) -> Option<String> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

fn describe_ref(r: &HyperTokenRef) -> String {
    let rev = &r.source_rev[..r.source_rev.len().min(8)];
    if r.is_path_unchanged() {
        format!("{}:{}", rev, r.lineno)
    } else {
        format!("{}:{}:{}", rev, r.path, r.lineno)
    }
}

fn describe(data: &HyperLineData) -> String {
    let mut s = describe_ref(&data.introduced);
    if let Some(pred) = &data.predecessor {
        s.push_str(&format!(" <= {}", describe_ref(pred)));
    }
    if let Some(marker) = &data.removal_marker {
        s.push_str(&format!(
            " !! {} removed {} ({} moved) from {}:{} first={}",
            &marker.source_rev[..marker.source_rev.len().min(8)],
            marker.num_removed,
            marker.num_moved,
            marker.path,
            marker.lineno,
            describe_ref(&marker.first_removed)
        ));
    }
    s
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "Usage: {} SYNTAX_REPO TIMELINE_REPO TIMELINE_REV PATH",
            args[0]
        );
        std::process::exit(1);
    }
    let syntax_repo = Repository::open(&args[1]).unwrap();
    let timeline_repo = Repository::open(&args[2]).unwrap();
    let timeline_commit = timeline_repo
        .revparse_single(&args[3])
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let path = &args[4];

    let meta = timeline_commit_to_meta(&timeline_commit);
    let syntax_commit = syntax_repo.find_commit(meta.syntax_rev).unwrap();
    println!(
        "source {} syntax {} timeline {}",
        meta.source_rev, meta.syntax_rev, meta.timeline_rev
    );

    let tokens = read_path(
        &syntax_repo,
        &syntax_commit.tree().unwrap(),
        &format!("files/{}", path),
    )
    .expect("no syntax file for path");
    let annotated = read_path(
        &timeline_repo,
        &timeline_commit.tree().unwrap(),
        &format!("annotated/{}", path),
    )
    .expect("no annotated file for path");

    let token_lines = token_file_lines(&tokens);
    let annotated_lines: Vec<&str> = annotated.lines().collect();
    if annotated_lines.len() != token_lines.len() + 1 {
        println!(
            "WARNING: {} tokens but {} annotated lines (expected tokens + 1)",
            token_lines.len(),
            annotated_lines.len()
        );
    }

    if let Some(sentinel) = annotated_lines.first() {
        println!(
            "{:>5} {:<40} | {}",
            0,
            "<file>",
            describe(&HyperLineData::parse(sentinel))
        );
    }
    for (i, line) in token_lines.iter().enumerate() {
        let token = split_token_line(line);
        let desc = annotated_lines
            .get(i + 1)
            .map(|a| describe(&HyperLineData::parse(a)))
            .unwrap_or_else(|| "???".to_string());
        let shown = format!("{} {}", token.context, token.token);
        println!("{:>5} {:<40} | {}", i + 1, shown, desc);
    }
}
