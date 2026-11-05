//! scip-typescript projects of a tree's JS environments (see `discover`): a
//! tsconfig.json per group of environments sharing a kind (their flavor, or
//! their flavor and global for a worker's or frame's) and a context (the
//! scripts they're loaded after, in order, and, for a flavor with a scope, a
//! generated shim declaring the scope's members as globals; see `scope_shim`),
//! whose roots (their tests) are its `scipModuleFiles`, modules whatever their
//! syntax, so that each one's own declarations are its own (as they are:
//! tests aren't loaded into each other's globals), as a project per
//! environment was ~5 times slower for Firefox; with its global kind's types
//! (see `rules::GlobalTypes`), the scripts it unwraps (`scipUnwrapFiles`;
//! see `rules::Rules::unwrap`), and the names its scripts can use as hooks
//! (`scipHookNames`: its tests' functions their harness calls by their
//! names); documenting (`scipOnlyFiles`) each script once
//! per kind, but those which other analysis already covers (ex: TypeScript
//! projects').  The documents' analyses are then merged as platforms' (a kind's
//! analysis directory each; see scripts/js-environments-analyze.sh), so that
//! a script in several kinds of environments (ex: an xpcshell test's and a
//! mochitest page's) has each one's definitions.  Pages' inline scripts
//! aren't documented yet.  Projects are written in batches by kind, for
//! scip-typescript processes each indexing a batch.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::json;
use tree_sitter::{Node, Parser};

use super::discover::{Discovery, Role};
use super::rules::{Flavor, Rules, Scope, glob_set};

/// The directory of the scope shims, which scip-indexer's `--shims` doesn't
/// write analysis files for (see `scope_shim`).
pub const SHIMS_DIR: &str = "__jsenv_shims__";

pub struct ProjectsOptions<'a> {
    pub files_root: &'a Path,
    pub out_dir: &'a Path,
    /// The flavors whose environments get projects (default: the manifests'
    /// flavors; not path flavors' or web-platform-tests', whose many tests
    /// would need grouping).
    pub flavors: Option<BTreeSet<String>>,
    /// The files other analysis covers.
    pub covered: &'a HashSet<String>,
    /// How many projects a batch has.
    pub batch_size: usize,
}

#[derive(Debug, Default)]
pub struct ProjectsSummary {
    /// Projects by kind.
    pub projects: BTreeMap<String, usize>,
    /// Documented files by kind.
    pub documents: BTreeMap<String, usize>,
    pub batches: usize,
}

/// A scope's members (see `rules::Scope`): its constructor's `this.name = ...`
/// assignments (in its body, not in functions in it), and its prototype's
/// members (`Constructor.prototype = { ... }`'s, and `Constructor.prototype.name
/// = ...`'s), but those whose names start with `_` (its private ones), each
/// once, its instances' first.
/// A script's top-level declarations' names (its functions', classes' and
/// variables'), unless it's a module by its syntax (whose declarations aren't
/// its global's): the names a harness can look up by name in its global, as
/// hooks (see scip-typescript's --hooks).
pub fn top_level_names(source: &str) -> Vec<String> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let root = tree.root_node();
    let mut names = vec![];
    for i in 0..root.named_child_count() {
        let Some(node) = root.named_child(i as u32) else {
            continue;
        };
        match node.kind() {
            "import_statement" | "export_statement" => return vec![],
            "function_declaration" | "generator_function_declaration" | "class_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    names.push(source[name.byte_range()].to_string());
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                for j in 0..node.named_child_count() {
                    if let Some(declarator) = node.named_child(j as u32)
                        && declarator.kind() == "variable_declarator"
                        && let Some(name) = declarator.child_by_field_name("name")
                        && name.kind() == "identifier"
                    {
                        names.push(source[name.byte_range()].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    names.sort();
    names.dedup();
    names
}

/// The identifiers a script mentions, of `names`.
fn mentioned_names<'a>(source: &str, names: &HashSet<&'a str>) -> HashSet<&'a str> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    let mut mentioned = HashSet::new();
    if parser.set_language(&language).is_err() {
        return mentioned;
    }
    let Some(tree) = parser.parse(source, None) else {
        return mentioned;
    };
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        if node.kind() == "identifier"
            && let Some(name) = names.get(&source[node.byte_range()])
        {
            mentioned.insert(*name);
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    mentioned
}

pub fn scope_members(source: &str, constructor: &str) -> (Vec<String>, Vec<String>) {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    if parser.set_language(&language).is_err() {
        return (vec![], vec![]);
    }
    let Some(tree) = parser.parse(source, None) else {
        return (vec![], vec![]);
    };
    let text = |n: Node| &source[n.byte_range()];
    let mut instance: Vec<String> = vec![];
    let mut prototype: Vec<String> = vec![];
    let push = |list: &mut Vec<String>, name: &str| {
        if !name.starts_with('_') && !list.iter().any(|n| n == name) {
            list.push(name.to_string());
        }
    };
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
        match node.kind() {
            "function_declaration"
                if node
                    .child_by_field_name("name")
                    .is_some_and(|n| text(n) == constructor) =>
            {
                // (Its body's `this.name = ...`s, but in functions in it.)
                let Some(body) = node.child_by_field_name("body") else {
                    continue;
                };
                let mut inner = vec![body];
                while let Some(n) = inner.pop() {
                    // (In reverse, so that they're popped in order.)
                    for i in (0..n.child_count()).rev() {
                        let Some(child) = n.child(i as u32) else {
                            continue;
                        };
                        if matches!(
                            child.kind(),
                            "function_declaration"
                                | "function_expression"
                                | "function"
                                | "method_definition"
                                | "class"
                        ) {
                            continue;
                        }
                        inner.push(child);
                    }
                    if n.kind() == "assignment_expression"
                        && let Some(left) = n.child_by_field_name("left")
                        && left.kind() == "member_expression"
                        && left
                            .child_by_field_name("object")
                            .is_some_and(|o| o.kind() == "this")
                        && let Some(property) = left.child_by_field_name("property")
                    {
                        push(&mut instance, text(property));
                    }
                }
            }
            "assignment_expression" => {
                let (Some(left), Some(right)) = (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ) else {
                    continue;
                };
                let prototype_of = format!("{}.prototype", constructor);
                if text(left) == prototype_of && right.kind() == "object" {
                    for i in 0..right.named_child_count() {
                        let Some(member) = right.named_child(i as u32) else {
                            continue;
                        };
                        let key = match member.kind() {
                            "pair" => member.child_by_field_name("key"),
                            "method_definition" => member.child_by_field_name("name"),
                            "shorthand_property_identifier" => Some(member),
                            _ => None,
                        };
                        if let Some(key) = key.filter(|k| {
                            k.kind() == "property_identifier"
                                || k.kind() == "shorthand_property_identifier"
                        }) {
                            push(&mut prototype, text(key));
                        }
                    }
                } else if left.kind() == "member_expression"
                    && left
                        .child_by_field_name("object")
                        .is_some_and(|o| text(o) == prototype_of)
                    && let Some(property) = left.child_by_field_name("property")
                {
                    push(&mut prototype, text(property));
                }
            }
            _ => {}
        }
    }
    prototype.retain(|name| !instance.contains(name));
    (instance, prototype)
}

/// A flavor's scope shim: a script declaring its scope's members as globals,
/// each a `var` whose value is the member (`new Constructor().name`,
/// `Constructor.prototype.name`, or a module's export), which scip-indexer's
/// `--shims` makes the uses of the global uses of the member, by the last
/// reference on its line.
pub fn scope_shim(flavor: &Flavor, scope: &Scope, source: &str) -> String {
    let (instance, prototype) = scope_members(source, &scope.constructor);
    let mut shim = format!(
        "// {}'s tests' scope: {}'s {}'s members, and its modules' exports, which\n\
         // its tests see as globals (generated by js-environments-projects; see\n\
         // mozsearch's tools/src/js_environments/projects.rs).\n",
        flavor.name, scope.file, scope.constructor
    );
    for name in &instance {
        shim += &format!("var {} = new {}().{};\n", name, scope.constructor, name);
    }
    for name in &prototype {
        if !instance.contains(name) {
            shim += &format!("var {} = {}.prototype.{};\n", name, scope.constructor, name);
        }
    }
    // (A `new ` URL's export is a class, whose instance the member is, ex:
    // browser-chrome's `Assert`.)
    for (name, url) in &scope.modules {
        let (instance, url) = match url.strip_prefix("new ") {
            Some(url) => (true, url),
            None => (false, url.as_str()),
        };
        let export = format!(
            "ChromeUtils.importESModule({}).{}",
            serde_json::to_string(url).unwrap(),
            name
        );
        if instance {
            shim += &format!("var {} = new ({})();\n", name, export);
        } else {
            shim += &format!("var {} = {};\n", name, export);
        }
    }
    shim
}

const SCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".jsm", ".cjs"];

/// How many roots (ex: tests) a project has at most, for its program's size.
const MAX_GROUP_ROOTS: usize = 200;

/// Write the environments' projects, batches and shims into `out_dir`
/// (`projects/<id>.json`, `batches/<kind>.<n>.lst`, `__jsenv_shims__/`).
pub fn write_projects(
    discovery: &Discovery,
    rules: &Rules,
    read: &dyn Fn(&str) -> Option<String>,
    opts: &ProjectsOptions,
) -> io::Result<ProjectsSummary> {
    let abs = |path: &str| opts.files_root.join(path).to_string_lossy().to_string();
    fs::create_dir_all(opts.out_dir.join("projects"))?;
    fs::create_dir_all(opts.out_dir.join("batches"))?;
    fs::create_dir_all(opts.out_dir.join(SHIMS_DIR))?;
    // The flavors' shims.
    let mut shims: BTreeMap<&str, PathBuf> = BTreeMap::new();
    for flavor in &rules.flavors {
        if let Some(scope) = &flavor.scope
            && let Some(source) = read(&scope.file)
        {
            let path = opts
                .out_dir
                .join(SHIMS_DIR)
                .join(format!("{}.js", flavor.name));
            fs::write(&path, scope_shim(flavor, scope, &source))?;
            shims.insert(&flavor.name, path);
        }
    }
    // (A flavor's own global: its rule's; web-platform-tests' is a window.)
    let flavor_global = |name: &str| -> Option<&str> {
        rules
            .flavors
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.global.as_str())
            .or_else(|| {
                rules
                    .path_flavors
                    .iter()
                    .find(|f| f.name == name)
                    .map(|f| f.global.as_str())
            })
            .or_else(|| (name == "wpt" && rules.wpt.is_some()).then_some("window"))
    };
    let selected = |name: &str| match &opts.flavors {
        Some(flavors) => flavors.contains(name),
        None => flavor_global(name).is_some(),
    };
    let mut summary = ProjectsSummary::default();
    // The environments, grouped by their kinds and contexts: the scripts
    // they're loaded after (and their global, and shim), each group's roots
    // (its tests, if they're JS) forced to be modules in its projects, so
    // that their own declarations are each one's (a test isn't loaded into
    // another's global), as a project per environment was ~25 times slower.
    // (In order of their first environments, for stable projects.)
    struct Group {
        kind: String,
        global: String,
        shim: Option<PathBuf>,
        context: Vec<String>,
        roots: Vec<String>,
    }
    let mut groups: Vec<Group> = vec![];
    let mut group_of: HashMap<(String, String, Vec<String>), usize> = HashMap::new();
    for env in &discovery.environments {
        if !selected(&env.flavor) {
            continue;
        }
        // (Its kind: its flavor, in its flavor's own global, else with its
        // global, ex: a mochitest's worker's, or a web-platform-test's worker
        // variant's, so that a script in both, ex: a `.any.js` test, has
        // each one's definitions.)
        let own_global = flavor_global(&env.flavor) == Some(env.global.as_str());
        let kind = if own_global {
            env.flavor.clone()
        } else {
            format!("{}-{}", env.flavor, env.global)
        };
        let scripts: Vec<&super::discover::Script> = env
            .scripts
            .iter()
            .filter(|s| {
                s.role != Role::Inline && SCRIPT_EXTENSIONS.iter().any(|e| s.path.ends_with(e))
            })
            .collect();
        // (Its root, if it's one of its JS scripts, ex: a test, or a worker's
        // script; not a page.)
        let root = scripts
            .iter()
            .find(|s| s.path == env.root && s.role == Role::Test)
            .map(|s| s.path.clone());
        let context: Vec<String> = scripts
            .iter()
            .filter(|s| Some(&s.path) != root.as_ref())
            .map(|s| s.path.clone())
            .collect();
        let shim = own_global
            .then(|| shims.get(env.flavor.as_str()).cloned())
            .flatten();
        let key = (kind.clone(), env.global.clone(), context.clone());
        let i = match group_of.get(&key) {
            Some(&i) if groups[i].roots.len() < MAX_GROUP_ROOTS => i,
            _ => {
                groups.push(Group {
                    kind: kind.clone(),
                    global: env.global.clone(),
                    shim,
                    context,
                    roots: vec![],
                });
                group_of.insert(key, groups.len() - 1);
                groups.len() - 1
            }
        };
        if let Some(root) = root
            && !groups[i].roots.contains(&root)
            && !groups[i].context.contains(&root)
        {
            groups[i].roots.push(root);
        }
    }
    // The names each context script can use as hooks, by kind (its project's
    // `scipHookNames`; see scip-typescript's --hooks, which finds those it
    // uses but doesn't declare, in the one project of its kind documenting
    // it, whose roots are only some of those it's loaded before): the
    // top-level names of its kind's environments' roots, which it mentions.
    let read = |path: &str| fs::read_to_string(opts.files_root.join(path)).ok();
    let roots: BTreeSet<&str> = groups
        .iter()
        .flat_map(|g| g.roots.iter().map(String::as_str))
        .collect();
    let root_names: HashMap<&str, Vec<String>> = roots
        .par_iter()
        .map(|root| {
            let names = read(root).map(|s| top_level_names(&s)).unwrap_or_default();
            (*root, names)
        })
        .collect();
    let all_root_names: HashSet<&str> = root_names.values().flatten().map(String::as_str).collect();
    let contexts: BTreeSet<&str> = groups
        .iter()
        .flat_map(|g| g.context.iter().map(String::as_str))
        .collect();
    let mentioned: HashMap<&str, HashSet<&str>> = contexts
        .par_iter()
        .map(|context| {
            let names = read(context)
                .map(|s| mentioned_names(&s, &all_root_names))
                .unwrap_or_default();
            (*context, names)
        })
        .collect();
    let mut hook_names: HashMap<(&str, &str), BTreeSet<&str>> = HashMap::new();
    for group in &groups {
        for context in &group.context {
            let Some(mentions) = mentioned.get(context.as_str()).filter(|m| !m.is_empty()) else {
                continue;
            };
            for root in &group.roots {
                for name in root_names.get(root.as_str()).into_iter().flatten() {
                    if mentions.contains(name.as_str()) {
                        hook_names
                            .entry((group.kind.as_str(), context.as_str()))
                            .or_default()
                            .insert(name.as_str());
                    }
                }
            }
        }
    }

    let unwrap_globs = glob_set(&rules.unwrap);
    let mut documented: HashSet<(String, String)> = HashSet::new();
    // (Each kind's projects, with their costs: their documented files' sizes.)
    let mut by_kind: BTreeMap<String, Vec<(PathBuf, u64)>> = BTreeMap::new();
    for (n, group) in groups.iter().enumerate() {
        let only: Vec<&String> = group
            .context
            .iter()
            .chain(&group.roots)
            .filter(|path| !opts.covered.contains(*path))
            .filter(|path| documented.insert((group.kind.clone(), path.to_string())))
            .collect();
        if only.is_empty() {
            continue;
        }
        let mut files: Vec<String> = vec![];
        let mut documented_files: Vec<String> = only.iter().map(|p| abs(p)).collect();
        if let Some(shim) = &group.shim {
            files.push(shim.to_string_lossy().to_string());
            // (Its document gives scip-indexer its aliases; see `scope_shim`.)
            documented_files.push(shim.to_string_lossy().to_string());
        }
        files.extend(group.context.iter().chain(&group.roots).map(|p| abs(p)));
        let types = rules.globals.get(&group.global);
        let mut compiler_options = json!({ "allowJs": true, "checkJs": false, "noEmit": true });
        if let Some(lib) = types.map(|t| &t.lib).filter(|l| !l.is_empty()) {
            compiler_options["lib"] = json!(lib);
        } else if types.and_then(|t| t.extends.as_ref()).is_none() {
            compiler_options["lib"] = json!(["ESNext"]);
        }
        let mut project = json!({
            "compilerOptions": compiler_options,
            "files": files,
            "scipOnlyFiles": documented_files,
            "scipModuleFiles": group.roots.iter().map(|p| abs(p)).collect::<Vec<_>>(),
        });
        let unwrap: Vec<String> = group
            .context
            .iter()
            .filter(|path| unwrap_globs.is_match(path.as_str()))
            .map(|p| abs(p))
            .collect();
        if !unwrap.is_empty() {
            project["scipUnwrapFiles"] = json!(unwrap);
        }
        let hooks: serde_json::Map<String, serde_json::Value> = only
            .iter()
            .filter(|path| group.context.contains(path))
            .filter_map(|path| {
                let names = hook_names.get(&(group.kind.as_str(), path.as_str()))?;
                Some((abs(path), json!(names)))
            })
            .collect();
        if !hooks.is_empty() {
            project["scipHookNames"] = serde_json::Value::Object(hooks);
        }
        if let Some(extends) = types.and_then(|t| t.extends.as_ref()) {
            project["extends"] = json!(abs(extends));
        }
        let path = opts.out_dir.join("projects").join(format!("{}.json", n));
        fs::write(&path, serde_json::to_string_pretty(&project).unwrap())?;
        *summary.projects.entry(group.kind.clone()).or_default() += 1;
        *summary.documents.entry(group.kind.clone()).or_default() += only.len();
        let cost: u64 = only
            .iter()
            .filter_map(|p| fs::metadata(opts.files_root.join(p)).ok())
            .map(|m| m.len())
            .sum();
        by_kind
            .entry(group.kind.clone())
            .or_default()
            .push((path, cost));
    }
    // (And batches.lst, the batches' files in order of their costs, the
    // costliest first, so that their processes end at about the same time.)
    let mut batches: Vec<(u64, PathBuf)> = vec![];
    for (kind, projects) in &by_kind {
        for (n, chunk) in projects.chunks(opts.batch_size.max(1)).enumerate() {
            let list: Vec<String> = chunk
                .iter()
                .map(|(p, _)| p.to_string_lossy().to_string())
                .collect();
            let path = opts
                .out_dir
                .join("batches")
                .join(format!("{}.{}.lst", kind, n));
            fs::write(&path, list.join("\n") + "\n")?;
            batches.push((chunk.iter().map(|(_, cost)| cost).sum(), path));
            summary.batches += 1;
        }
    }
    batches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let list: Vec<String> = batches
        .iter()
        .map(|(_, p)| p.to_string_lossy().to_string())
        .collect();
    fs::write(opts.out_dir.join("batches.lst"), list.join("\n") + "\n")?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_level_names() {
        let script = r#"
function testSteps() { function inner() {} }
function* gen() {}
var a = 1, b;
let { destructured } = obj;
const c = () => {};
class K {}
add_task(async function test_task() {});
"#;
        assert_eq!(
            top_level_names(script),
            vec!["K", "a", "b", "c", "gen", "testSteps"]
        );
        // (A module's declarations aren't its global's.)
        assert!(top_level_names("export function f() {}\nfunction g() {}\n").is_empty());
    }

    #[test]
    fn test_scope_members() {
        let source = r#"
function testScope(aTester) {
  this.__tester = aTester;
  var self = this;
  this.ok = function test_ok(condition, name) {};
  this.is = function test_is(a, b, name) {
    this.notAMember = 1;
  };
  this.info = function test_info(name) {};
}
testScope.prototype = {
  __done: true,
  EventUtils: {},
  add_task(properties, func = properties) {},
  info: null,
};
testScope.prototype.waitForExplicitFinish = function () {};
"#;
        let (instance, prototype) = scope_members(source, "testScope");
        assert_eq!(instance, vec!["ok", "is", "info"]);
        assert_eq!(
            prototype,
            vec!["EventUtils", "add_task", "waitForExplicitFinish"]
        );
    }
}
