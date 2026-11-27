use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use clap::Args;
use serde_json::{Value, json};

use super::interface::{PipelineCommand, PipelineValues, TextFile};
use crate::abstract_server::{AbstractServer, Result};
use crate::rendered_docs::escape;

/// The MIME type of `landings`' HTML, which the query page shows as it is.
pub const LANDINGS_MIME: &str = "application/vnd.searchfox.landings+html";

/// The tree's recent landings, by stack (the commits of a bug, or a commit
/// without one), newest first, each linked to its review (its bug's explore
/// page; see `try_review::landed_review_name`), with facets by subsystem,
/// directory, type and author: the `landings:DAYS` query term's, which the
/// tree's `/explore/` page is (see the web-server's `explore` route).  And,
/// with `--interesting` (`interesting-landings:DAYS`), how much more a
/// token-centric review shows of each than a line diff does (see
/// `landing_score`), by which they're sorted instead: ex: code moved to
/// another file and changed on the way, which a line diff shows as removed
/// and added, but a review as moved, with its changes marked.  And with
/// `--pushes` (`try-pushes:DAYS`, `interesting-try-pushes:DAYS`), in a tree
/// importing try pushes, its imported pushes, each linked to its review: the
/// reviews someone could do now.
#[derive(Debug, Args)]
pub struct Landings {
    /// How many days of landings, up to the tree's last.
    #[clap(value_parser, default_value_t = crate::format::LANDINGS_DAYS)]
    days: u32,

    /// Weigh their changes (slower: each commit's rev-summary).
    #[clap(long, value_parser)]
    interesting: bool,

    /// The imported try pushes, rather than the landings.
    #[clap(long, value_parser)]
    pushes: bool,
}

#[derive(Debug)]
pub struct LandingsCommand {
    pub args: Landings,
}

/// A stack of landed commits (see `Landings`): its key (`bug/123`, or
/// `rev/<rev>` for a commit without a bug, or a try push's `push/<name>`),
/// its commits (the data's, oldest first), and its score (see
/// `landing_score`).
struct Stack<'a> {
    key: String,
    commits: Vec<&'a Value>,
    score: u64,
}

/// How much more a token-centric review shows of a stack's commits than a
/// line diff does, by their tokens' changes (see `explore::CommitStats`):
/// the tokens they moved (within or between files) or evolved (ex: renamed),
/// which a line diff shows as removed and added, and three times more for
/// those of symbols moved from other files and changed on the way (which a
/// review shows against where they were, with the changes marked).  And
/// what it says of them, for the stack's badges.
fn landing_score(commits: &[&Value]) -> (u64, Vec<String>) {
    let number = |value: &Value| value.as_u64().unwrap_or(0);
    let mut moved = 0;
    let mut changed = 0;
    let mut cross = 0;
    let mut cross_edited = 0;
    let mut edited_tokens = 0;
    for commit in commits {
        let stats = &commit["stats"];
        if stats.is_null() {
            continue;
        }
        moved += number(&stats["moved"]) + number(&stats["evolved"]);
        changed += number(&stats["added"])
            + number(&stats["removed"])
            + number(&stats["moved"])
            + number(&stats["evolved"]);
        for symbol in stats["moves"].as_array().into_iter().flatten() {
            cross += 1;
            if number(&symbol["added"]) > 0 {
                cross_edited += 1;
                edited_tokens += number(&symbol["moved"]);
            }
        }
    }
    let mut badges = vec![];
    if moved > 0 {
        badges.push(format!(
            "{} tokens moved ({}% of its changes)",
            moved,
            moved * 100 / changed.max(1)
        ));
    }
    if cross > 0 {
        badges.push(match cross_edited {
            0 => format!(
                "{} symbol{} moved from other files",
                cross,
                if cross == 1 { "" } else { "s" }
            ),
            edited => format!(
                "{} symbol{} moved from other files, {} changed on the way",
                cross,
                if cross == 1 { "" } else { "s" },
                edited
            ),
        });
    }
    (moved + 3 * edited_tokens, badges)
}

/// A path's values of a hierarchical facet, each with its ancestors (ex:
/// "dom/clients/api" for "dom", "dom/clients" and "dom/clients/api").
fn levels(path: &str, parts: usize) -> Vec<(String, String)> {
    let mut values = vec![];
    let mut so_far = String::new();
    for part in path.split('/').take(parts) {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        values.push((so_far.clone(), part.to_string()));
    }
    values
}

/// The facets' values' tree, for `FacetView`s: by value, its name, how many
/// stacks have it, and its nested values.
#[derive(Default)]
struct Level {
    name: String,
    count: u32,
    nested: BTreeMap<String, Level>,
}

fn facet_views(level: Level) -> Vec<crate::cmd_pipeline::facets::FacetValueView> {
    let mut values: Vec<(String, Level)> = level.nested.into_iter().collect();
    values.sort_by(|(a_value, a), (b_value, b)| {
        b.count.cmp(&a.count).then_with(|| a_value.cmp(b_value))
    });
    values
        .into_iter()
        .map(
            |(value, level)| crate::cmd_pipeline::facets::FacetValueView {
                value,
                title: level.name.clone(),
                name: level.name.clone(),
                count: level.count,
                values: facet_views(level),
                ..Default::default()
            },
        )
        .collect()
}

#[async_trait]
impl PipelineCommand for LandingsCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        use crate::cmd_pipeline::facets::FacetView;

        let tree = server.tree_info()?.name;
        let days = self.args.days.clamp(1, crate::format::MAX_LANDINGS_DAYS);
        let interesting = self.args.interesting;
        let pushes = self.args.pushes;
        // (Cached by the index's identity: a tree's landings are its
        // index's.  Not its imported pushes, which come and go.)
        let data = match pushes {
            true => server.landings(days, interesting, true).await?,
            false => {
                crate::step_cache::cached(
                    server.cache_identity(),
                    "landings",
                    &format!("{}{}", days, if interesting { "i" } else { "" }),
                    || async { server.landings(days, interesting, false).await },
                )
                .await?
            }
        };
        let imports_try_pushes = data["imports_try_pushes"].as_bool().unwrap_or(false);
        // (A page of the listing: its days, whether it weighs them, and
        // which listing, in a tree importing try pushes, whose page is its
        // pushes' unless `landed=1`.)
        let page = |days: u32, interesting: bool| {
            let mut params = vec![format!("days={}", days)];
            if interesting {
                params.push("interesting=1".to_string());
            }
            if imports_try_pushes && !pushes {
                params.push("landed=1".to_string());
            }
            format!("/{}/explore/?{}", tree, params.join("&"))
        };
        let commits: Vec<&Value> = data["commits"].as_array().into_iter().flatten().collect();

        // (Its stacks: each bug's commits, by its first bug, and each other
        // commit alone, by their last landing, newest first.)
        let mut by_key: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
        let mut order: Vec<String> = vec![];
        for commit in &commits {
            let key = match (
                commit["push"].as_str(),
                commit["bugs"].as_array().and_then(|bugs| bugs.first()),
            ) {
                (Some(push), _) => format!("push/{}", push),
                (None, Some(bug)) => format!("bug/{}", bug.as_str().unwrap_or_default()),
                (None, None) => format!("rev/{}", commit["rev"].as_str().unwrap_or_default()),
            };
            if !by_key.contains_key(&key) {
                order.push(key.clone());
            }
            by_key.entry(key).or_default().push(commit);
        }
        let last = |stack: &Stack| {
            stack
                .commits
                .last()
                .and_then(|commit| commit["landed"].as_str())
                .unwrap_or_default()
                .to_string()
        };
        let mut stacks: Vec<Stack> = order
            .into_iter()
            .map(|key| {
                let commits = by_key.remove(&key).unwrap_or_default();
                let score = landing_score(&commits).0;
                Stack {
                    key,
                    commits,
                    score,
                }
            })
            .collect();
        stacks.sort_by(|a, b| match interesting {
            true => b.score.cmp(&a.score).then_with(|| last(b).cmp(&last(a))),
            false => last(b).cmp(&last(a)),
        });

        // (Each stack's facets: its files' subsystems, directories (two
        // levels) and types, and its authors.)
        let mut roots: BTreeMap<&'static str, Level> = BTreeMap::new();
        let mut items = String::new();
        for stack in &stacks {
            let files: BTreeSet<&str> = stack
                .commits
                .iter()
                .flat_map(|commit| commit["files"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .collect();
            let authors: BTreeSet<&str> = stack
                .commits
                .iter()
                .filter_map(|commit| commit["author"].as_str())
                .collect();
            // (Each facet's chains of values, from the top, ex: a file's
            // directory and its parent's.)
            let mut chains: BTreeMap<&'static str, BTreeSet<Vec<(String, String)>>> =
                BTreeMap::new();
            for path in &files {
                let (_, _, subsystem) = server.path_facet_info(path);
                if let Some(subsystem) = subsystem {
                    chains
                        .entry("subsystem")
                        .or_default()
                        .insert(levels(subsystem.as_str(), usize::MAX));
                }
                if let Some((dir, _)) = path.rsplit_once('/') {
                    chains
                        .entry("directory")
                        .or_default()
                        .insert(levels(dir, 2));
                }
                chains
                    .entry("type")
                    .or_default()
                    .insert(crate::cmd_pipeline::facets::type_value_path(path));
            }
            for author in &authors {
                chains
                    .entry("author")
                    .or_default()
                    .insert(vec![(author.to_string(), author.to_string())]);
            }
            // (Counted once for the stack, each value with its ancestors.)
            let mut facets: BTreeMap<&str, Vec<String>> = BTreeMap::new();
            for (key, set) in chains {
                let mut seen: BTreeSet<String> = BTreeSet::new();
                for chain in set {
                    let mut level = roots.entry(key).or_default();
                    for (value, name) in chain {
                        level = level.nested.entry(value.clone()).or_default();
                        level.name = name;
                        if seen.insert(value.clone()) {
                            level.count += 1;
                            facets.entry(key).or_default().push(value);
                        }
                    }
                }
            }
            items.push_str(&stack_html(&tree, stack, &facets, interesting));
        }

        let mut facet_views_out = vec![];
        for (key, label) in [
            ("subsystem", "Subsystem"),
            ("directory", "Directory"),
            ("type", "Type"),
            ("author", "Author"),
        ] {
            if let Some(level) = roots.remove(key) {
                facet_views_out.push(FacetView::new(key, label.to_string(), facet_views(level)));
            }
        }
        let landed_range = format!(
            "{} to {}",
            escape(
                &data["since"]
                    .as_str()
                    .unwrap_or_default()
                    .replace('T', " ")
                    .replace('Z', " UTC")
            ),
            escape(
                &data["until"]
                    .as_str()
                    .unwrap_or_default()
                    .replace('T', " ")
                    .replace('Z', " UTC")
            )
        );
        let days_links: Vec<String> = [1, 7, 30]
            .iter()
            .map(|n| match *n == days {
                true => format!("<b>{} day{}</b>", n, if *n == 1 { "" } else { "s" }),
                false => format!(
                    r#"<a href="{href}">{n} day{s}</a>"#,
                    href = escape(&page(*n, interesting)),
                    n = n,
                    s = if *n == 1 { "" } else { "s" },
                ),
            })
            .collect();
        let mode = match interesting {
            true => format!(
                r#"Sorted by how much more their reviews show than a line diff (ex: Phabricator's) does: the tokens they moved or renamed, which a line diff shows as removed and added, and more for code moved from other files and changed on the way, which a review shows against where it was.  <a href="{href}">Newest first instead</a>."#,
                href = escape(&page(days, false)),
            ),
            false => format!(
                r#"Newest first.  <a class="landings-interesting" href="{href}">Find the interesting ones</a>: which their reviews show the most of that a line diff (ex: Phabricator's) doesn't, ex: code moved and changed on the way (slower)."#,
                href = escape(&page(days, true)),
            ),
        };
        // (In a tree importing try pushes, a link to the other listing.)
        let other = match (imports_try_pushes, pushes) {
            (true, true) => format!(
                r#"  Or <a class="landings-other" href="/{tree}/explore/?days={days}&amp;landed=1">its branch's recent landings</a>."#,
                tree = escape(&tree),
                days = days,
            ),
            (true, false) => format!(
                r#"  Or <a class="landings-other" href="/{tree}/explore/?days={days}">its try pushes to review</a>."#,
                tree = escape(&tree),
                days = days,
            ),
            _ => String::new(),
        };
        let (title, about) = match pushes {
            true => (
                "Try pushes to review",
                format!(
                    "The {stacks} try pushes {tree} has imported whose newest commits are from {range}, with their {commits} commits, each linked to its review.",
                    stacks = stacks.len(),
                    tree = escape(&tree),
                    range = landed_range,
                    commits = commits.len(),
                ),
            ),
            false => (
                "Recent landings",
                format!(
                    "The {stacks} stacks (each bug's commits, or a commit without one) of the {commits} commits {tree}'s branch landed from {range}{truncated}, each linked to its review.",
                    stacks = stacks.len(),
                    commits = commits.len(),
                    tree = escape(&tree),
                    range = landed_range,
                    truncated = match data["truncated"].as_bool().unwrap_or(false) {
                        true => " (its newest ones, as there were more)",
                        false => "",
                    },
                ),
            ),
        };
        let html = format!(
            r#"<div class="landings" id="landings"><h2>{title}</h2><p class="landings-about">{about}  {mode}{other}</p><p class="landings-days">Over the last {days_links}.</p><div class="facet-items landings-items">{items}</div></div>"#,
            title = title,
            about = about,
            mode = mode,
            other = other,
            days_links = days_links.join(" · "),
            items = items,
        );
        Ok(PipelineValues::TextFile(TextFile {
            mime_type: LANDINGS_MIME.to_string(),
            contents: html,
            transient: false,
            facets: Some(json!({ "facets": facet_views_out, "noun": "stack", "nouns": "stacks" })),
        }))
    }
}

/// A stack's item in the listing: its heading (its bug, linked to its
/// review, or its commit), its summary, its authors, its last landing, its
/// Phabricator revisions (as Phabricator shows them), its badges (if
/// `interesting`; see `landing_score`), and its commits.
fn stack_html(
    tree: &str,
    stack: &Stack,
    facets: &BTreeMap<&str, Vec<String>>,
    interesting: bool,
) -> String {
    let first = stack.commits.first().copied().unwrap_or(&Value::Null);
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    let (link, name) = match stack.key.split_once('/') {
        Some(("bug", bug)) => (
            format!("/{}/explore/bug/{}", tree, bug),
            format!("Bug {}", bug),
        ),
        // (A try push's page, its review.)
        Some(("push", push)) => {
            let (what, short) = crate::try_import::push_label(push);
            (
                format!("/{}/{}", tree, crate::try_import::push_url_path(push)),
                format!("{}{} {}", what[..1].to_uppercase(), &what[1..], short),
            )
        }
        _ => {
            let rev = text(&first["rev"]);
            (
                format!("/{}/commit/{}", tree, rev),
                rev.chars().take(12).collect(),
            )
        }
    };
    let authors: BTreeSet<String> = stack
        .commits
        .iter()
        .map(|commit| text(&commit["author"]))
        .collect();
    let phabs: BTreeSet<u64> = stack
        .commits
        .iter()
        .filter_map(|commit| commit["phab"].as_u64())
        .collect();
    let last = stack
        .commits
        .last()
        .map(|commit| text(&commit["landed"]))
        .unwrap_or_default();
    let backouts = stack
        .commits
        .iter()
        .filter(|commit| commit["backout"].as_bool().unwrap_or(false))
        .count();
    let mut badges: Vec<String> = match interesting {
        true => landing_score(&stack.commits).1,
        false => vec![],
    };
    if backouts > 0 {
        badges.push(match backouts {
            1 => "a backout".to_string(),
            n => format!("{} backouts", n),
        });
    }
    let commits: Vec<String> = stack
        .commits
        .iter()
        .map(|commit| {
            let rev = text(&commit["rev"]);
            format!(
                r#"<li><a href="/{tree}/commit/{rev}">{short}</a> {summary}</li>"#,
                tree = escape(tree),
                rev = escape(&rev),
                short = escape(&rev.chars().take(12).collect::<String>()),
                summary = escape(&text(&commit["summary"])),
            )
        })
        .collect();
    format!(
        r#"<section class="landing" data-path="{key}" data-size="{score}" data-facets="{facets}"><h4 class="landing-head"><a class="landing-link" href="{link}">{name}</a> <span class="landing-summary">{summary}</span></h4><p class="landing-meta">{count} · {authors} · {when} {last}{phabs}</p>{badges}{commits}</section>"#,
        key = escape(&stack.key),
        score = stack.score,
        facets = escape(&serde_json::to_string(facets).unwrap_or_default()),
        link = escape(&link),
        name = escape(&name),
        summary = escape(&text(&first["summary"])),
        count = match stack.commits.len() {
            1 => "1 commit".to_string(),
            n => format!("{} commits", n),
        },
        authors = escape(&authors.into_iter().collect::<Vec<_>>().join(", ")),
        // (A try push's commits haven't landed.)
        when = match stack.key.starts_with("push/") {
            true => "committed",
            false => "landed",
        },
        last = escape(&last.replace('T', " ").replace('Z', " UTC")),
        phabs = match phabs.is_empty() {
            true => String::new(),
            false => format!(
                " · on Phabricator: {}",
                phabs
                    .iter()
                    .map(|d| format!(
                        r#"<a class="landing-phab" href="https://phabricator.services.mozilla.com/D{d}">D{d}</a>"#
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
        badges = match badges.is_empty() {
            true => String::new(),
            false => format!(
                r#"<p class="landing-badges">{}</p>"#,
                badges
                    .iter()
                    .map(|badge| format!(r#"<span class="landing-badge">{}</span>"#, escape(badge)))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        },
        // (A few commits listed; more, folded.)
        commits = match commits.len() {
            0..=LISTED_COMMITS => format!(r#"<ol class="landing-commits">{}</ol>"#, commits.join("")),
            n => format!(
                r#"<details class="landing-more"><summary>Its {} commits</summary><ol class="landing-commits">{}</ol></details>"#,
                n,
                commits.join("")
            ),
        },
    )
}

/// The most commits a stack's item lists unfolded.
const LISTED_COMMITS: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    /// A stack's score: its tokens moved or renamed, and three times more
    /// for those of symbols moved from other files and changed on the way.
    #[test]
    fn test_landing_score() {
        let moved = json!({ "stats": {
            "added": 10, "removed": 30, "moved": 50, "evolved": 10,
            "moves": [
                { "from_path": "a.cpp", "from": "A", "to_path": "b.cpp", "to": "A", "moved": 40, "added": 5 },
                { "from_path": "a.cpp", "from": "B", "to_path": "b.cpp", "to": "B", "moved": 20, "added": 0 },
            ],
        }});
        let plain = json!({ "stats": { "added": 100, "removed": 20 } });
        let unweighed = json!({});
        let (score, badges) = landing_score(&[&moved, &plain, &unweighed]);
        assert_eq!(score, 60 + 3 * 40);
        assert_eq!(
            badges,
            vec![
                "60 tokens moved (27% of its changes)".to_string(),
                "2 symbols moved from other files, 1 changed on the way".to_string(),
            ]
        );
        assert_eq!(landing_score(&[&plain]), (0, vec![]));
    }

    #[test]
    fn test_levels() {
        assert_eq!(
            levels("dom/clients/api", 2),
            vec![
                ("dom".to_string(), "dom".to_string()),
                ("dom/clients".to_string(), "clients".to_string()),
            ]
        );
        assert_eq!(levels("Core/DOM", usize::MAX).len(), 2);
    }
}
