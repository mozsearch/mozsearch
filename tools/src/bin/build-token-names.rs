// Build a tree's token-name index (see `file_format::token_names`): the names
// of its history's token journals at the head of its branch, into the index's
// `token-names/` directory.  A tree without a history gets none.
//
// Usage: build-token-names CONFIG_FILE TREE_NAME

use std::path::Path;
use std::time::Instant;

use git2::{ObjectType, TreeWalkMode, TreeWalkResult};
use tools::file_format::config::load;
use tools::file_format::token_names::write_index;
use tools::hyperblame::token_blame::TreeHistory;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (config_file, tree_name) = (&args[1], &args[2]);
    let config = load(config_file, false, Some(tree_name), None, None);
    let tree = &config.trees[tree_name];
    let Some(history_path) = &tree.paths.history_path else {
        println!("{} has no history, so no token-name index", tree_name);
        return;
    };
    let start = Instant::now();
    let history = TreeHistory::open(history_path, tree.paths.git_branch.as_deref()).unwrap();
    let Some(head) = history.head_timeline_commit() else {
        println!(
            "{}'s history has no head, so no token-name index",
            tree_name
        );
        return;
    };
    let root = head.tree().unwrap();
    let mut names = vec![];
    if let Ok(tokens) = root.get_path(Path::new("tokens")) {
        let tokens = history.timeline.find_tree(tokens.id()).unwrap();
        // (tokens/AB/CD/TOKEN.ndjson, and segments in TOKEN.ndjson.d/, which
        // aren't walked into.)
        tokens
            .walk(TreeWalkMode::PreOrder, |dir, entry| {
                let name = entry.name().unwrap_or("");
                match entry.kind() {
                    Some(ObjectType::Tree) if name.ends_with(".ndjson.d") => TreeWalkResult::Skip,
                    Some(ObjectType::Blob) if dir.matches('/').count() == 2 => {
                        if let Some(token) = name.strip_suffix(".ndjson") {
                            names.push(token.to_string());
                        }
                        TreeWalkResult::Ok
                    }
                    _ => TreeWalkResult::Ok,
                }
            })
            .unwrap();
    }
    let listed = start.elapsed();
    let count = names.len();
    let dir = Path::new(&tree.paths.index_path).join("token-names");
    write_index(&dir, names).unwrap();
    println!(
        "{}: {} token names (listed in {:.1} s, written in {:.1} s) in {}",
        tree_name,
        count,
        listed.as_secs_f64(),
        (start.elapsed() - listed).as_secs_f64(),
        dir.display()
    );
}
