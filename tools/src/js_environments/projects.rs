//! scip-typescript projects of a tree's JS environments (see `discover`): a
//! tsconfig.json per group of environments sharing a kind (their flavor, or
//! their flavor and global for a worker's or frame's) and a context (the
//! scripts they're loaded after, in order, and, for a flavor with a scope, a
//! generated shim declaring the scope's members as globals; see `scope_shim`),
//! whose roots (their tests) are its `scipModuleFiles`, modules whatever their
//! syntax, so that each one's own declarations are its own (as they are:
//! tests aren't loaded into each other's globals), as a project per
//! environment was ~5 times slower for Firefox; with its global kind's types
//! (see `rules::GlobalTypes`); documenting (`scipOnlyFiles`) each script once
//! per kind, but those which other analysis already covers (ex: TypeScript
//! projects').  The documents' analyses are then merged as platforms' (a kind's
//! analysis directory each; see scripts/js-environments-analyze.sh), so that
//! a script in several kinds of environments (ex: an xpcshell test's and a
//! mochitest page's) has each one's definitions.  Pages' inline scripts
//! aren't documented yet.  Projects are written in batches by kind, for
//! scip-typescript processes each indexing a batch.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;
use tree_sitter::{Node, Parser};

use super::discover::{Discovery, Role};
use super::rules::{Flavor, Rules, Scope};

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
    let selected = |name: &str| match &opts.flavors {
        Some(flavors) => flavors.contains(name),
        None => rules.flavors.iter().any(|f| f.name == name),
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
    let mut group_of: std::collections::HashMap<(String, String, Vec<String>), usize> =
        std::collections::HashMap::new();
    for env in &discovery.environments {
        if !selected(&env.flavor) {
            continue;
        }
        let flavor = rules.flavors.iter().find(|f| f.name == env.flavor);
        let own_global = flavor.is_some_and(|f| f.global == env.global);
        let kind = if own_global || flavor.is_none() && env.parent.is_none() {
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
    let mut documented: HashSet<(String, String)> = HashSet::new();
    let mut by_kind: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
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
        if let Some(extends) = types.and_then(|t| t.extends.as_ref()) {
            project["extends"] = json!(abs(extends));
        }
        let path = opts.out_dir.join("projects").join(format!("{}.json", n));
        fs::write(&path, serde_json::to_string_pretty(&project).unwrap())?;
        *summary.projects.entry(group.kind.clone()).or_default() += 1;
        *summary.documents.entry(group.kind.clone()).or_default() += only.len();
        by_kind.entry(group.kind.clone()).or_default().push(path);
    }
    for (kind, projects) in &by_kind {
        for (n, chunk) in projects.chunks(opts.batch_size.max(1)).enumerate() {
            let list: Vec<String> = chunk
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            fs::write(
                opts.out_dir
                    .join("batches")
                    .join(format!("{}.{}.lst", kind, n)),
                list.join("\n") + "\n",
            )?;
            summary.batches += 1;
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

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
