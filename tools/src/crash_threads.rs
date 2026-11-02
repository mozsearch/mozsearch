//! What a crash report's threads are, and what each was doing when the crash
//! happened, for the crash view (see crash_stack.rs): by the rules of
//! crash-threads.toml (config_defaults', unless the config repo has its own;
//! see the file for what they say), each thread's kind (by its name, or its
//! stack: ex: "DOM Worker", "graphics thread", "TaskController pool"), its
//! owner (ex: a Bugzilla component, or a third party's module), its state
//! (idle at its normal wait for work, waiting elsewhere, active, notable, ex:
//! shutting down, or unknown), and its frames' roles: its waits at its
//! stack's top, and its layers (its start, its event loop, its pool), which
//! the view collapses.

use std::collections::HashSet;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesToml {
    #[serde(default)]
    wait: Vec<WaitToml>,
    #[serde(default)]
    layer: Vec<LayerToml>,
    #[serde(default)]
    notable: Vec<NotableToml>,
    #[serde(default)]
    defaults: DefaultsToml,
    #[serde(default)]
    modules: ModulesToml,
    #[serde(default)]
    thread: Vec<ThreadToml>,
    #[serde(default)]
    places: PlacesToml,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacesToml {
    #[serde(default)]
    libraries: Vec<String>,
    #[serde(default)]
    generic: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitToml {
    kind: String,
    functions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerToml {
    name: String,
    functions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotableToml {
    state: String,
    functions: Vec<String>,
    #[serde(default)]
    called_by: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsToml {
    #[serde(default)]
    idle: Vec<String>,
    #[serde(default)]
    loops: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModulesToml {
    #[serde(default)]
    gecko: Vec<String>,
    #[serde(default)]
    system: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadToml {
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    stack: Vec<String>,
    kind: String,
    owner: String,
    #[serde(default)]
    idle: Vec<String>,
    loops: Option<usize>,
    #[serde(default)]
    notable: Vec<NotableToml>,
}

/// Patterns of names, "*" being any characters, as one regexp.
struct Patterns(Option<Regex>);

impl Patterns {
    fn new(patterns: &[String]) -> Result<Self, String> {
        if patterns.is_empty() {
            return Ok(Patterns(None));
        }
        let alternatives = patterns
            .iter()
            .map(|pattern| {
                pattern
                    .split('*')
                    .map(regex::escape)
                    .collect::<Vec<_>>()
                    .join(".*")
            })
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&format!("^(?s:{})$", alternatives))
            .map(|re| Patterns(Some(re)))
            .map_err(|e| e.to_string())
    }

    fn matches(&self, name: &str) -> bool {
        self.0.as_ref().is_some_and(|re| re.is_match(name))
    }
}

struct Notable {
    state: String,
    functions: Patterns,
    called_by: Option<Patterns>,
}

impl Notable {
    fn new(toml: &NotableToml) -> Result<Self, String> {
        Ok(Notable {
            state: toml.state.clone(),
            functions: Patterns::new(&toml.functions)?,
            called_by: match toml.called_by.is_empty() {
                true => None,
                false => Some(Patterns::new(&toml.called_by)?),
            },
        })
    }
}

struct ThreadRule {
    names: Vec<String>,
    name_patterns: Patterns,
    stack: Patterns,
    kind: String,
    owner: String,
    idle: Patterns,
    loops: usize,
    notable: Vec<Notable>,
}

impl ThreadRule {
    /// Whether the thread's name (without a number, ex: "#3") is one of the
    /// rule's: for a name the crash reporter truncated (ex: "TaskCon~ller"),
    /// one it could be.
    fn matches_name(&self, name: &str) -> bool {
        let Some((prefix, suffix)) = name.split_once('~') else {
            return self.name_patterns.matches(name);
        };
        !prefix.is_empty()
            && self
                .names
                .iter()
                .any(|pattern| match pattern.split_once('*') {
                    None => pattern.starts_with(prefix) && pattern.ends_with(suffix),
                    Some((literal, _)) => {
                        prefix.starts_with(literal) || literal.starts_with(prefix)
                    }
                })
    }
}

/// The rules of crash-threads.toml (see the module's documentation).
pub struct ThreadRules {
    fingerprint: String,
    waits: Vec<(String, Patterns)>,
    layers: Vec<(String, Patterns)>,
    notable: Vec<Notable>,
    idle: Patterns,
    loops: Patterns,
    gecko: HashSet<String>,
    system: HashSet<String>,
    threads: Vec<ThreadRule>,
    libraries: Patterns,
    generic: Patterns,
}

/// A frame of a thread's stack, as the rules see it: its function's name (as
/// the report has it, with its arguments), its module, and whether it was
/// found by scanning the stack (which may not be the thread's).
pub struct RuleFrame<'a> {
    pub function: Option<&'a str>,
    pub module: Option<&'a str>,
    pub scanned: bool,
}

/// What a thread is, and was doing (see the module's documentation): its
/// kind (a rule's, or else "Gecko thread", "system thread" or a third
/// party's, ex: "libgallium-25.0.7.so thread"), its owner, whether a rule
/// described it, and its state.
#[derive(Clone, Debug, Serialize)]
pub struct ThreadClass {
    pub kind: String,
    pub owner: String,
    pub described: bool,
    pub state: ThreadState,
}

/// What a thread was doing when the crash happened: idle at its normal wait
/// for work (its event loop's, or its pool's), waiting elsewhere (on a
/// condition variable, a lock, an event, or I/O), active (running its own
/// code), something notable (ex: "shutting down", "nested event loop"),
/// unknown (its stack doesn't say), or crashed (the crashing thread).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", content = "what", rename_all = "kebab-case")]
pub enum ThreadState {
    Idle,
    Waiting(String),
    Active,
    Notable(String),
    Unknown,
    Crashed,
}

impl ThreadState {
    /// As the view says it (ex: "waiting (condition variable)").
    pub fn label(&self) -> String {
        match self {
            ThreadState::Idle => "idle".to_string(),
            ThreadState::Waiting(kind) => format!("waiting ({})", kind),
            ThreadState::Active => "active".to_string(),
            ThreadState::Notable(what) => what.clone(),
            ThreadState::Unknown => "unknown".to_string(),
            ThreadState::Crashed => "crashed".to_string(),
        }
    }

    /// Its facet's value (see `slug`; ex: "waiting", "shutting-down").
    pub fn slug(&self) -> String {
        match self {
            ThreadState::Waiting(_) => "waiting".to_string(),
            ThreadState::Notable(what) => slug(what),
            state => state.label(),
        }
    }

    /// What it means, for its tooltip.
    pub fn description(&self) -> &'static str {
        match self {
            ThreadState::Idle => {
                "At its normal wait for work (its event loop's, or its pool's): its frames are its wait and its infrastructure"
            }
            ThreadState::Waiting(_) => "Blocked, but not at its normal wait for work",
            ThreadState::Active => "Running its own code",
            ThreadState::Notable(_) => "Doing something its rules call out",
            ThreadState::Unknown => "Its stack doesn't say (ex: its frames have no symbols)",
            ThreadState::Crashed => "The thread which crashed",
        }
    }
}

/// A frame's role in its thread's stack: a wait (at the stack's top, of a
/// thread which isn't the crashing one), a layer's (its name), an idle
/// thread's idle point, or the thread's own.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameRole {
    Wait,
    Layer(String),
    Idle,
    Own,
}

/// A name as a facet's value: lowercase, its other characters than letters
/// and digits as dashes (ex: "graphics-thread", "taskcontroller-pool").
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// A frame's function's name without its arguments (and its qualifiers, ex:
/// " const"), as the rules' patterns are of: Rust's `<Type>::method` as
/// `Type::method` (but not `<T as Trait>::method`), and a thunk's (ex:
/// "{virtual override thunk({offset(-32)}, F(...))}") as its function's.
pub fn rule_name(function: &str) -> String {
    let mut name = function.trim();
    if let Some(inner) = name
        .strip_prefix("{virtual override thunk({")
        .or_else(|| name.strip_prefix("{virtual thunk({"))
        .and_then(|rest| rest.split_once("}, "))
        .and_then(|(_, rest)| rest.strip_suffix(")}"))
    {
        name = inner;
    }
    for qualifier in [" const", " volatile", " &&", " &", " noexcept"] {
        if let Some(rest) = name.strip_suffix(qualifier)
            && rest.ends_with(')')
        {
            name = rest;
        }
    }
    // (The arguments' parentheses, matched from the end, but not a name
    // which is all parentheses, ex: "(anonymous namespace)".)
    if name.ends_with(')') {
        let mut depth = 0;
        for (at, c) in name.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        if at > 0 {
                            name = name[..at].trim_end();
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    if name.starts_with('<') {
        let mut depth = 0;
        for (at, c) in name.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = &name[1..at];
                        if !inner.contains(" as ") && name[at + 1..].starts_with("::") {
                            return format!("{}{}", inner, &name[at + 1..]);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    name.to_string()
}

/// A thread's name without its number (ex: "DOM Worker" for "DOM Worker #3").
pub fn rule_thread_name(name: &str) -> &str {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    match trimmed.strip_suffix('#') {
        Some(rest) if trimmed.len() < name.len() => rest.trim_end(),
        _ => name.trim(),
    }
}

impl ThreadRules {
    /// The rules of crash-threads.toml's text.
    pub fn parse(text: &str) -> Result<ThreadRules, String> {
        let toml: RulesToml = toml::from_str(text).map_err(|e| e.to_string())?;
        let fingerprint = format!("{:016x}", crate::crash_sets::fnv1a(text.as_bytes()));
        let named = |name: &String, functions: &[String]| -> Result<(String, Patterns), String> {
            Ok((name.clone(), Patterns::new(functions)?))
        };
        Ok(ThreadRules {
            fingerprint,
            waits: toml
                .wait
                .iter()
                .map(|wait| named(&wait.kind, &wait.functions))
                .collect::<Result<_, _>>()?,
            layers: toml
                .layer
                .iter()
                .map(|layer| named(&layer.name, &layer.functions))
                .collect::<Result<_, _>>()?,
            notable: toml
                .notable
                .iter()
                .map(Notable::new)
                .collect::<Result<_, _>>()?,
            idle: Patterns::new(&toml.defaults.idle)?,
            loops: Patterns::new(&toml.defaults.loops)?,
            gecko: toml.modules.gecko.into_iter().collect(),
            system: toml.modules.system.into_iter().collect(),
            libraries: Patterns::new(&toml.places.libraries)?,
            generic: Patterns::new(&toml.places.generic)?,
            threads: toml
                .thread
                .iter()
                .map(|rule| {
                    Ok(ThreadRule {
                        names: rule.names.clone(),
                        name_patterns: Patterns::new(&rule.names)?,
                        stack: Patterns::new(&rule.stack)?,
                        kind: rule.kind.clone(),
                        owner: rule.owner.clone(),
                        idle: Patterns::new(&rule.idle)?,
                        loops: rule.loops.unwrap_or(1),
                        notable: rule
                            .notable
                            .iter()
                            .map(Notable::new)
                            .collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }

    /// The rules of crash-threads.toml's `text`, compiled once for each text
    /// (so that a changed file's rules are its new ones), or None if they
    /// don't parse (which it logs).
    pub fn cached(text: &str) -> Option<std::sync::Arc<ThreadRules>> {
        use std::sync::{Arc, Mutex};
        type Cached = Option<(String, Option<Arc<ThreadRules>>)>;
        static CACHED: Mutex<Cached> = Mutex::new(None);
        let mut cached = CACHED.lock().unwrap();
        if let Some((cached_text, rules)) = cached.as_ref()
            && cached_text == text
        {
            return rules.clone();
        }
        let rules = match ThreadRules::parse(text) {
            Ok(rules) => Some(Arc::new(rules)),
            Err(e) => {
                error!("crash-threads.toml: {}", e);
                None
            }
        };
        *cached = Some((text.to_string(), rules.clone()));
        rules
    }

    /// A hash of the rules' text, for caching what they classify (see
    /// `crash_sets`).
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn wait_kind(&self, name: &str) -> Option<&str> {
        self.waits
            .iter()
            .find(|(_, patterns)| patterns.matches(name))
            .map(|(kind, _)| kind.as_str())
    }

    /// Whether the directory `place` (ex: "third_party/sqlite3/src") is a
    /// library's, whose code a busy thread's place is above (see
    /// crash-threads.toml's `[places]`).
    pub fn is_library(&self, place: &str) -> bool {
        self.libraries.matches(place)
    }

    /// Whether a call to `function` (as a report has it, see `rule_name`) is
    /// generic application code (ex: an operation's dispatch, a directory's
    /// iteration), which a busy thread's place and its stack skip, as they
    /// do libraries' (see crash-threads.toml's `[places]`).
    pub fn is_generic(&self, function: &str) -> bool {
        self.generic.matches(&rule_name(function))
    }

    fn layer(&self, name: &str) -> Option<&str> {
        self.layers
            .iter()
            .find(|(_, patterns)| patterns.matches(name))
            .map(|(layer, _)| layer.as_str())
    }

    /// The layer whose frame a call to `function` (as a report has it, see
    /// `rule_name`) is, if it's one's (ex: of a call inlined in a layer's
    /// frame, which may be the thread's own code).
    pub fn layer_of(&self, function: &str) -> Option<&str> {
        self.layer(&rule_name(function))
    }

    /// What the thread named `name` is and was doing, by its stack's
    /// `frames` (innermost first; see `RuleFrame`), and each frame's role;
    /// the crashing thread's state is `Crashed`, and its frames at the top
    /// aren't waits.
    pub fn classify(
        &self,
        name: Option<&str>,
        frames: &[RuleFrame],
        crashing: bool,
    ) -> (ThreadClass, Vec<FrameRole>) {
        let names: Vec<String> = frames
            .iter()
            .map(|frame| frame.function.map(rule_name).unwrap_or_default())
            .collect();
        let unknown = |i: usize| {
            let name = &names[i];
            name.is_empty() || name == "?" || name.starts_with("<unknown")
        };
        // (The frames which aren't scanned, by their positions.)
        let known: Vec<usize> = (0..frames.len()).filter(|i| !frames[*i].scanned).collect();
        let thread_name = name.map(rule_thread_name).filter(|name| !name.is_empty());
        let rule = thread_name
            .and_then(|name| {
                self.threads
                    .iter()
                    .find(|rule| !rule.names.is_empty() && rule.matches_name(name))
            })
            .or_else(|| {
                self.threads
                    .iter()
                    .find(|rule| known.iter().any(|i| rule.stack.matches(&names[*i])))
            });

        // The waits at the top (and unsymbolized frames among them, if a wait
        // follows them).
        let mut wait: Option<&str> = None;
        let mut waits_end = 0;
        if !crashing {
            for (at, i) in known.iter().enumerate() {
                if unknown(*i) {
                    continue;
                }
                match self.wait_kind(&names[*i]) {
                    Some(kind) => {
                        wait = Some(kind);
                        waits_end = at + 1;
                    }
                    None => break,
                }
            }
        }
        let rest = &known[waits_end..];
        let below: Vec<usize> = rest.iter().copied().filter(|i| !unknown(*i)).collect();

        // A notable frame, ex: a nested event loop's, or shutting down, which
        // stays expanded (`expanded`, by their positions), with its caller
        // if the rule says which it is.
        let mut notable: Option<String> = None;
        let mut expanded: Vec<usize> = vec![];
        'notables: for n in rule
            .map_or(&[][..], |rule| &rule.notable[..])
            .iter()
            .chain(&self.notable)
        {
            for (at, i) in below.iter().enumerate() {
                if !n.functions.matches(&names[*i]) {
                    continue;
                }
                if let Some(called_by) = &n.called_by
                    && !below
                        .get(at + 1)
                        .is_some_and(|outer| called_by.matches(&names[*outer]))
                {
                    continue;
                }
                notable = Some(n.state.clone());
                expanded.push(*i);
                if n.called_by.is_some() {
                    expanded.extend(below.get(at + 1));
                }
                break 'notables;
            }
        }
        // (Of a nested event loop, its loops but the outermost.)
        let loops: Vec<usize> = below
            .iter()
            .copied()
            .filter(|i| self.loops.matches(&names[*i]))
            .collect();
        if notable.is_none() && loops.len() > rule.map_or(1, |rule| rule.loops) {
            notable = Some("nested event loop".to_string());
            expanded.extend(&loops[..loops.len() - 1]);
        }

        let idle_point = |i: usize| {
            self.idle.matches(&names[i]) || rule.is_some_and(|rule| rule.idle.matches(&names[i]))
        };
        let infrastructure = |i: usize| self.layer(&names[i]).is_some() || idle_point(i);
        let state = if crashing {
            ThreadState::Crashed
        } else if let Some(notable) = notable {
            ThreadState::Notable(notable)
        } else if known.iter().all(|i| unknown(*i)) {
            ThreadState::Unknown
        } else if let Some(kind) = wait {
            // (Or a system library's own thread, all of whose frames are the
            // system's, waiting for its work, ex: macOS's NSEventThread in
            // its run loop, or an audio framework's workers.)
            let system_only = below.iter().all(|i| {
                frames[*i]
                    .module
                    .is_some_and(|module| self.system.contains(module))
            });
            // (Or its wait's caller is its kind's own idle point, whatever is
            // below it, ex: a frame Windows' identical code folding misnamed;
            // nested loops and notable frames were found before.)
            let at_idle_point = below
                .first()
                .is_some_and(|i| rule.is_some_and(|rule| rule.idle.matches(&names[*i])));
            match !below.is_empty()
                && (at_idle_point || below.iter().all(|i| infrastructure(*i)) || system_only)
            {
                true => ThreadState::Idle,
                false => ThreadState::Waiting(kind.to_string()),
            }
        } else {
            // (Unsymbolized system frames at the top over its idle point are
            // its wait, ex: libc's epoll_wait without symbols.)
            let top = known
                .iter()
                .take_while(|i| {
                    unknown(**i)
                        && frames[**i]
                            .module
                            .is_none_or(|module| self.system.contains(module))
                })
                .count();
            if top > 0
                && below.first().is_some_and(|i| idle_point(*i))
                && below.iter().all(|i| infrastructure(*i))
            {
                waits_end = top;
                ThreadState::Idle
            } else if !below.is_empty()
                && below
                    .iter()
                    .all(|i| self.layer(&names[*i]).is_some() && !idle_point(*i))
            {
                // (Nothing but its start and loops.)
                ThreadState::Unknown
            } else {
                ThreadState::Active
            }
        };

        // The frames' roles: unknown and scanned frames have those of the
        // known frames around them, if they're the same (and below a
        // thread's start, its start's).
        let mut roles: Vec<Option<FrameRole>> = vec![None; frames.len()];
        for (at, i) in known.iter().enumerate() {
            if at < waits_end {
                roles[*i] = Some(FrameRole::Wait);
            } else if unknown(*i) {
                continue;
            } else if expanded.contains(i) {
                roles[*i] = Some(FrameRole::Own);
            } else if let Some(layer) = self.layer(&names[*i]) {
                roles[*i] = Some(FrameRole::Layer(layer.to_string()));
            } else if state == ThreadState::Idle {
                roles[*i] = Some(FrameRole::Idle);
            } else {
                roles[*i] = Some(FrameRole::Own);
            }
        }
        let mut filled: Vec<FrameRole> = Vec::with_capacity(frames.len());
        for i in 0..frames.len() {
            if let Some(role) = &roles[i] {
                filled.push(role.clone());
                continue;
            }
            let above = filled.last();
            let below = roles[i + 1..].iter().flatten().next();
            filled.push(match (above, below) {
                (Some(above), Some(below)) if above == below => above.clone(),
                (Some(FrameRole::Layer(layer)), None) => FrameRole::Layer(layer.clone()),
                (_, _) if state == ThreadState::Idle => FrameRole::Idle,
                _ => FrameRole::Own,
            });
        }

        let (kind, owner, described) = match rule {
            Some(rule) => (rule.kind.clone(), rule.owner.clone(), true),
            None => {
                let modules: Vec<&str> = below.iter().filter_map(|i| frames[*i].module).collect();
                match modules.iter().find(|module| {
                    !self.gecko.contains(**module) && !self.system.contains(**module)
                }) {
                    Some(module) => (
                        format!("{} thread", module),
                        format!("Third-party: {}", module),
                        false,
                    ),
                    None if modules.iter().any(|module| self.gecko.contains(*module)) => {
                        ("Gecko thread".to_string(), "Gecko".to_string(), false)
                    }
                    None => ("system thread".to_string(), "System".to_string(), false),
                }
            }
        };
        (
            ThreadClass {
                kind,
                owner,
                described,
                state,
            },
            filled,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"
[[wait]]
kind = "condition variable"
functions = ["mozilla::detail::ConditionVariableImpl::wait*", "pthread_cond_wait*", "std::sys::sync::condvar::*::wait*"]

[[wait]]
kind = "event"
functions = ["syscall", "mach_msg"]

[[wait]]
kind = "I/O"
functions = ["epoll_wait"]

[[layer]]
name = "thread start"
functions = ["start_thread", "__clone*", "std::sys::*thread_start", "std::sys::backtrace::__rust_begin_short_backtrace*"]

[[layer]]
name = "NSPR"
functions = ["_pt_root"]

[[layer]]
name = "XPCOM event loop"
functions = ["nsThread::ThreadFunc", "NS_ProcessNextEvent", "nsThread::ProcessNextEvent", "mozilla::ThreadEventQueue::GetEvent"]

[[layer]]
name = "Rayon"
functions = ["rayon_core::registry::*"]

[[layer]]
name = "worker event loop"
functions = ["mozilla::dom::workerinternals::(anonymous namespace)::WorkerThreadPrimaryRunnable::Run", "mozilla::dom::WorkerPrivate::DoRunLoop"]

[[layer]]
name = "Chromium IPC thread"
functions = ["base::MessagePumpLibevent::Run", "base::Thread::ThreadMain", "ThreadFunc"]

[[notable]]
state = "shutting down"
functions = ["nsThread::Shutdown*"]

[defaults]
idle = ["mozilla::ThreadEventQueue::GetEvent", "nsThread::ProcessNextEvent"]
loops = ["nsThread::ProcessNextEvent"]

[modules]
gecko = ["XUL"]
system = ["libc.so.6", "libsystem_kernel.dylib", "libsystem_pthread.dylib", "CoreFoundation", "AppKit"]

[[thread]]
names = ["DOM Worker"]
kind = "DOM Worker"
owner = "Core: DOM: Workers"
loops = 2
idle = ["mozilla::dom::WorkerPrivate::DoRunLoop"]
[[thread.notable]]
state = "shutting down"
functions = ["NS_ProcessPendingEvents"]
called_by = ["mozilla::dom::workerinternals::(anonymous namespace)::WorkerThreadPrimaryRunnable::Run"]

[[thread]]
names = ["TaskController"]
kind = "TaskController pool"
owner = "Core: XPCOM"

[[thread]]
names = ["StyleThread"]
kind = "Stylo worker"
owner = "Core: CSS"
idle = ["rayon_core::registry::WorkerThread::wait_until_cold"]

[[thread]]
names = ["IPC I/O Child"]
kind = "IPC I/O thread"
owner = "Core: IPC"
idle = ["base::MessagePumpLibevent::Run"]

[[thread]]
stack = ["angle::AsyncWorkerPool::threadLoop"]
kind = "ANGLE worker"
owner = "Core: Graphics"
"#;

    fn frames<'a>(functions: &'a [&'a str]) -> Vec<RuleFrame<'a>> {
        functions
            .iter()
            .map(|function| RuleFrame {
                function: (!function.is_empty()).then_some(*function),
                module: Some(if function.is_empty() {
                    "libc.so.6"
                } else {
                    "XUL"
                }),
                scanned: false,
            })
            .collect()
    }

    fn classify(name: Option<&str>, functions: &[&str]) -> (ThreadClass, Vec<FrameRole>) {
        ThreadRules::parse(RULES)
            .unwrap()
            .classify(name, &frames(functions), false)
    }

    const XPCOM_LOOP: &[&str] = &[
        "pthread_cond_wait",
        "mozilla::detail::ConditionVariableImpl::wait(mozilla::detail::MutexImpl&)",
        "mozilla::ThreadEventQueue::GetEvent(bool, mozilla::TimeDuration*)",
        "nsThread::ProcessNextEvent(bool, bool*)",
        "NS_ProcessNextEvent(nsIThread*, bool)",
        "nsThread::ThreadFunc(void*)",
        "_pt_root",
        "start_thread",
        "__clone3",
    ];

    #[test]
    fn test_rule_name() {
        for (function, name) in [
            (
                "nsThread::ProcessNextEvent(bool, bool*)",
                "nsThread::ProcessNextEvent",
            ),
            (
                "mozilla::dom::workerinternals::(anonymous namespace)::WorkerThreadPrimaryRunnable::Run()",
                "mozilla::dom::workerinternals::(anonymous namespace)::WorkerThreadPrimaryRunnable::Run",
            ),
            (
                "(anonymous namespace)::ThreadFunc(void*)",
                "(anonymous namespace)::ThreadFunc",
            ),
            ("Foo::operator()(int) const", "Foo::operator()"),
            (
                "<std::sys::sync::condvar::futex::Condvar>::wait",
                "std::sys::sync::condvar::futex::Condvar::wait",
            ),
            (
                "<alloc::boxed::Box<F> as core::ops::function::FnOnce<A>>::call_once",
                "<alloc::boxed::Box<F> as core::ops::function::FnOnce<A>>::call_once",
            ),
            (
                "{virtual override thunk({offset(-32)}, mozilla::net::nsSocketTransportService::Run())}",
                "mozilla::net::nsSocketTransportService::Run",
            ),
            ("?", "?"),
        ] {
            assert_eq!(rule_name(function), name, "{}", function);
        }
        assert_eq!(rule_thread_name("DOM Worker #3"), "DOM Worker");
        assert_eq!(rule_thread_name("StyleThread#12"), "StyleThread");
        assert_eq!(rule_thread_name("IPC I/O Child"), "IPC I/O Child");
        assert_eq!(slug("TaskController pool"), "taskcontroller-pool");
        assert_eq!(slug("Third-party: libfoo.so.1"), "third-party-libfoo-so-1");
    }

    #[test]
    fn test_classify() {
        // A pool's thread waiting for its next event: idle, its waits and
        // its layers, by its name (truncated, as the crash reporter does).
        let (class, roles) = classify(Some("TaskCon~ller #4"), XPCOM_LOOP);
        assert_eq!(class.kind, "TaskController pool");
        assert_eq!(class.owner, "Core: XPCOM");
        assert!(class.described);
        assert_eq!(class.state, ThreadState::Idle);
        assert_eq!(roles[0], FrameRole::Wait);
        assert_eq!(roles[1], FrameRole::Wait);
        assert_eq!(roles[2], FrameRole::Layer("XPCOM event loop".to_string()));
        assert_eq!(roles[6], FrameRole::Layer("NSPR".to_string()));
        assert_eq!(roles[8], FrameRole::Layer("thread start".to_string()));

        // Waiting elsewhere: its own frames between its wait and its loop.
        let mut waiting = XPCOM_LOOP.to_vec();
        waiting.splice(2..3, ["mozilla::Foo::WaitForBar()", "mozilla::Foo::Run()"]);
        let (class, roles) = classify(Some("TaskController #1"), &waiting);
        assert_eq!(
            class.state,
            ThreadState::Waiting("condition variable".to_string())
        );
        assert_eq!(class.state.label(), "waiting (condition variable)");
        assert_eq!(class.state.slug(), "waiting");
        assert_eq!(roles[2], FrameRole::Own);
        assert_eq!(roles[3], FrameRole::Own);
        assert_eq!(roles[4], FrameRole::Layer("XPCOM event loop".to_string()));

        // Active: no wait at the top.  And the crashing thread crashed, its
        // top frames its own.
        let (class, roles) = classify(Some("TaskController #1"), &waiting[2..]);
        assert_eq!(class.state, ThreadState::Active);
        assert_eq!(roles[0], FrameRole::Own);
        let (class, roles) = ThreadRules::parse(RULES).unwrap().classify(
            Some("TaskController #1"),
            &frames(XPCOM_LOOP),
            true,
        );
        assert_eq!(class.state, ThreadState::Crashed);
        assert_eq!(roles[0], FrameRole::Own);
        assert_eq!(roles[2], FrameRole::Layer("XPCOM event loop".to_string()));

        // A nested event loop: more loops than its kind has.
        let mut nested = XPCOM_LOOP.to_vec();
        nested.splice(
            5..5,
            [
                "mozilla::Foo::Spin()",
                "nsThread::ProcessNextEvent(bool, bool*)",
                "NS_ProcessNextEvent(nsIThread*, bool)",
            ],
        );
        let (class, roles) = classify(Some("TaskController"), &nested);
        assert_eq!(
            class.state,
            ThreadState::Notable("nested event loop".to_string())
        );
        assert_eq!(class.state.slug(), "nested-event-loop");
        // (Its inner loop stays expanded, but not the outermost.)
        assert_eq!(roles[3], FrameRole::Own);
        assert_eq!(roles[6], FrameRole::Layer("XPCOM event loop".to_string()));

        // A DOM Worker idle in its own loop in its thread's (2 loops), or
        // shutting down: its primary runnable processing its pending events.
        let worker = [
            "pthread_cond_wait",
            "mozilla::dom::WorkerPrivate::DoRunLoop(JSContext*)",
            "mozilla::dom::workerinternals::(anonymous namespace)::WorkerThreadPrimaryRunnable::Run()",
            "nsThread::ProcessNextEvent(bool, bool*)",
            "NS_ProcessNextEvent(nsIThread*, bool)",
            "nsThread::ThreadFunc(void*)",
            "start_thread",
        ];
        let (class, _) = classify(Some("DOM Worker"), &worker);
        assert_eq!(class.state, ThreadState::Idle);
        let mut shutting_down = worker.to_vec();
        shutting_down[1] = "NS_ProcessPendingEvents(nsIThread*, mozilla::TimeDuration)";
        let (class, roles) = classify(Some("DOM Worker"), &shutting_down);
        assert_eq!(
            class.state,
            ThreadState::Notable("shutting down".to_string())
        );
        // (Its notable frame and its caller stay expanded.)
        assert_eq!(roles[1], FrameRole::Own);
        assert_eq!(roles[2], FrameRole::Own);
        assert_eq!(roles[3], FrameRole::Layer("XPCOM event loop".to_string()));
        // (Not if something else calls it.)
        shutting_down[2] = "mozilla::dom::Other::Run()";
        let (class, _) = classify(Some("DOM Worker"), &shutting_down);
        assert_eq!(
            class.state,
            ThreadState::Waiting("condition variable".to_string())
        );
        // (Waiting at its kind's idle point is idle, whatever is below it,
        // ex: a misnamed frame.)
        let (class, _) = classify(
            Some("DOM Worker"),
            &[
                "pthread_cond_wait",
                "mozilla::dom::WorkerPrivate::DoRunLoop(JSContext*)",
                "mozilla::detail::RunnableMethodArguments<>::apply<nsObserverService>()",
                "start_thread",
            ],
        );
        assert_eq!(class.state, ThreadState::Idle);

        // A Rust pool's thread (Rust's names), idle at its wait in its layers.
        let (class, roles) = classify(
            Some("StyleThread#3"),
            &[
                "syscall",
                "<std::sys::sync::condvar::futex::Condvar>::wait",
                "<rayon_core::registry::WorkerThread>::wait_until_cold",
                "<rayon_core::registry::ThreadBuilder>::run",
                "std::sys::backtrace::__rust_begin_short_backtrace::<style::global_style_data::thread_spawn::{closure#0}, ()>",
                "<std::sys::thread::unix::Thread>::new::thread_start",
                "start_thread",
            ],
        );
        assert_eq!(class.state, ThreadState::Idle);
        assert_eq!(roles[1], FrameRole::Wait);
        assert_eq!(roles[2], FrameRole::Layer("Rayon".to_string()));

        // Unsymbolized system frames over its idle point: its wait.
        let (class, roles) = classify(
            Some("IPC I/O Child"),
            &[
                "",
                "base::MessagePumpLibevent::Run()",
                "base::Thread::ThreadMain()",
                "ThreadFunc(void*)",
                "start_thread",
            ],
        );
        assert_eq!(class.state, ThreadState::Idle);
        assert_eq!(roles[0], FrameRole::Wait);
        // Only its start and loops: unknown.
        let (class, _) = classify(
            Some("TaskController"),
            &["", "nsThread::ThreadFunc(void*)", "start_thread"],
        );
        assert_eq!(class.state, ThreadState::Unknown);
        let (class, _) = classify(Some("TaskController"), &["", ""]);
        assert_eq!(class.state, ThreadState::Unknown);

        // A system library's thread waiting, all of its frames the system's:
        // idle (but not if Gecko's code called the wait).
        let system = |function: &'static str, module: &'static str| RuleFrame {
            function: Some(function),
            module: Some(module),
            scanned: false,
        };
        let rules = ThreadRules::parse(RULES).unwrap();
        let (class, roles) = rules.classify(
            Some("com.apple.NSEventThread"),
            &[
                system("mach_msg", "libsystem_kernel.dylib"),
                system("__CFRunLoopServiceMachPort", "CoreFoundation"),
                system("_NSEventThread", "AppKit"),
                system("_pthread_start", "libsystem_pthread.dylib"),
            ],
            false,
        );
        assert_eq!(class.state, ThreadState::Idle);
        assert_eq!(
            (class.kind.as_str(), class.owner.as_str()),
            ("system thread", "System")
        );
        assert_eq!(roles[1], FrameRole::Idle);
        let (class, _) = rules.classify(
            Some("com.apple.NSEventThread"),
            &[
                system("mach_msg", "libsystem_kernel.dylib"),
                system("mozilla::Foo::Bar()", "XUL"),
                system("_pthread_start", "libsystem_pthread.dylib"),
            ],
            false,
        );
        assert_eq!(class.state, ThreadState::Waiting("event".to_string()));

        // A thread described by its stack, and those no rule describes: the
        // owner whose module its frames are in.
        let (class, _) = classify(
            None,
            &[
                "pthread_cond_wait",
                "angle::AsyncWorkerPool::threadLoop()",
                "start_thread",
            ],
        );
        assert_eq!(class.kind, "ANGLE worker");
        let (class, _) = classify(
            Some("Link Monitor"),
            &["pthread_cond_wait", "mozilla::Foo::Run()", "start_thread"],
        );
        assert_eq!(
            (class.kind.as_str(), class.owner.as_str(), class.described),
            ("Gecko thread", "Gecko", false)
        );
        let third_party = [
            RuleFrame {
                function: Some("pthread_cond_wait"),
                module: Some("libc.so.6"),
                scanned: false,
            },
            RuleFrame {
                function: Some("util_queue_thread_func"),
                module: Some("libgallium.so"),
                scanned: false,
            },
            RuleFrame {
                function: Some("start_thread"),
                module: Some("libc.so.6"),
                scanned: false,
            },
        ];
        let (class, _) = ThreadRules::parse(RULES).unwrap().classify(
            Some("firefo:traceq0"),
            &third_party,
            false,
        );
        assert_eq!(
            (class.kind.as_str(), class.owner.as_str()),
            ("libgallium.so thread", "Third-party: libgallium.so")
        );

        // Scanned frames aren't the rules', and unknown frames between a
        // layer's frames are the layer's.
        let mut scanned = frames(XPCOM_LOOP);
        scanned.insert(
            3,
            RuleFrame {
                function: Some("mozilla::Foo::Bar()"),
                module: Some("XUL"),
                scanned: true,
            },
        );
        scanned.insert(
            5,
            RuleFrame {
                function: None,
                module: Some("XUL"),
                scanned: false,
            },
        );
        let (class, roles) =
            ThreadRules::parse(RULES)
                .unwrap()
                .classify(Some("TaskController"), &scanned, false);
        assert_eq!(class.state, ThreadState::Idle);
        assert_eq!(roles[3], FrameRole::Layer("XPCOM event loop".to_string()));
        assert_eq!(roles[5], FrameRole::Layer("XPCOM event loop".to_string()));
    }

    /// config_defaults/crash-threads.toml parses, and no kind's facet value
    /// is a state's.
    #[test]
    fn test_default_rules() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../config_defaults/crash-threads.toml"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("Skipping: no config_defaults/crash-threads.toml here");
            return;
        };
        let rules = ThreadRules::parse(&text).unwrap();
        let states = ["idle", "waiting", "active", "unknown", "crashed"];
        for rule in &rules.threads {
            assert!(
                !states.contains(&slug(&rule.kind).as_str()),
                "{}",
                rule.kind
            );
        }
        for notable in rules
            .notable
            .iter()
            .chain(rules.threads.iter().flat_map(|rule| &rule.notable))
        {
            assert!(
                !states.contains(&slug(&notable.state).as_str()),
                "{}",
                notable.state
            );
        }
        let (class, _) = rules.classify(Some("DOM Worker"), &frames(XPCOM_LOOP), false);
        assert_eq!(class.state, ThreadState::Idle);
    }
}
