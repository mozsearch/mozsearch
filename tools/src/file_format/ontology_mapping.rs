use serde::{Deserialize, Serialize};
use ustr::{ustr, Ustr, UstrMap};

#[derive(Deserialize)]
pub struct OntologyMappingConfig {
    #[serde(default)]
    pub pretty: UstrMap<OntologyRule>,
    #[serde(default)]
    pub types: UstrMap<OntologyType>,
}

#[derive(Deserialize)]
pub struct OntologyRule {
    /// When specified, treats the given symbol/identifier as an nsIRunnable::Run
    /// style method where overrides should be treated as runnables and have
    /// ontology slots allocated to point to the concrete constructors.
    #[serde(default)]
    pub runnable: bool,
    /// Given a base class, find all of its subclasses which are expected to be
    /// inner classes and label the outer class that contains them.  This mainly
    /// exists for detecting cycle collection where we have an inner class that
    /// is glued to the containing class by macros.
    pub label_containing_class: Option<OntologyLabelContainingClass>,
    /// Given a base class, find all of its subclasses which are expected to be
    /// inner classes, walk out to the containing class, then process all of its
    /// fields' uses to see if any of them have a contextsym matching the given
    /// "context_sym_suffix" and apply the labels if so.
    ///
    /// This very much exists for labeling cycle collected fields where the
    /// traversal/unlink logic lives on an inner class that's glued to the
    /// outer class with macros.  This could potentially be less hacky in terms
    /// of the suffix mechanism, but there's not a clear upside at this point.
    pub label_containing_class_field_uses: Option<OntologyLabelContainingClassFieldUses>,
    /// Given a class that can be directly used as a field on objects, whenever
    /// we see a field with this type, label the owning class with the given
    /// labels.
    pub label_owning_class: Option<OntologyLabelOwningClass>,
    /// Labels that we always apply to the class.
    #[serde(default)]
    pub labels: Vec<Ustr>,
}

#[derive(Deserialize)]
pub struct OntologyLabelContainingClassFieldUses {
    #[serde(default)]
    pub labels: Vec<OntologyContextSymLabelRule>,
}

#[derive(Deserialize)]
pub struct OntologyLabelContainingClass {
    #[serde(default)]
    pub labels: Vec<OntologyAlwaysLabelRule>,
}

#[derive(Clone, Deserialize)]
pub struct OntologyLabelOwningClass {
    #[serde(default)]
    pub labels: Vec<OntologyAlwaysLabelRule>,
}

#[derive(Clone, Deserialize)]
pub struct OntologyAlwaysLabelRule {
    pub label: Ustr,
}

#[derive(Deserialize)]
pub struct OntologyContextSymLabelRule {
    pub context_sym_suffix: Ustr,
    pub label: Ustr,
}

#[derive(Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OntologyType {
    Pointer(OntologyTypePointer),
    Value,
    Variant,
    Nothing,
}

#[derive(Eq, PartialEq, Deserialize)]
pub struct OntologyTypePointer {
    pub kind: OntologyPointerKind,
    #[serde(default)]
    pub arg_index: u32,
}

#[derive(Eq, PartialEq, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OntologyPointerKind {
    Strong,
    Unique,
    Weak,
    Raw,
    Ref,
    Contains,
}

pub struct OntologyMappingIngestion {
    pub config: OntologyMappingConfig,
}

impl OntologyMappingIngestion {
    pub fn new(config_str: &str) -> Result<Self, String> {
        let config: OntologyMappingConfig =
            toml::from_str(config_str).map_err(|err| err.to_string())?;

        Ok(OntologyMappingIngestion { config })
    }
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
enum TypeParseState {
    /// We're parsing a type.
    Typish,
    /// We've most recently seen a ">" and now don't care about whitespace and
    /// just expect to see either ">" or ","
    Closing,
}

#[derive(Default)]
struct ShoddyType {
    is_const: bool,
    is_pointer: bool,
    is_ref: bool,
    is_tag: bool,
    is_nothing: bool,
    identifier: String,
    args: Vec<ShoddyType>,
    // We set this to true if we've already put it in the results list,
    consumed: bool,
}

impl OntologyMappingConfig {
    /// Shoddily parse the type, looking up the types we find, seeing if this
    /// type seems to represent a pointer type.  If we identify a pointer type,
    /// we return the pointer kind (strong, unique, weak, raw) and the pretty
    /// identifier for the type which we can probably look up.
    ///
    /// The motivating situation here is:
    /// - For structured C++ fields, the "typesym" we have is just something
    ///   like "T_RefPtr" or "T_InitializedOnce" (plus namespace), and lacks
    ///   the information we actually need, so we currently need to parse it
    ///   out of the "type".  In the future we can potentially enhance the C++
    ///   indexer, but that's a non-trivial amount of work and out-of-scope at
    ///   the current time.
    /// - We just want to know the class being pointed at and the kind of the
    ///   pointer; we don't really care or want any extra type-magic like
    ///   if "InitializedOnce" or "Maybe" is used at this time.
    /// - In the future we may want to understand extra type annotations that
    ///   indicate if something is nullable, when it's initialized, etc. so it's
    ///   nice to support that.
    /// - We want to be able to distinguish "SafeRefPtr" from "RefPtr" which a
    ///   regex based solution might get tripped up on.
    ///
    /// So this is:
    /// - Intended to be slightly better than a regexp for being able to apply
    ///   simple type rules based on what we see in the type signature.
    /// - Not intended to grow or become more sophisticated than being able to
    ///   build a simple tree with very simple rules.  We have access to clang
    ///   and all the info it has, and we should just use that as the next step.
    ///
    /// TODO: Distinguish a ref to a strong pointer from just a ref.
    /// - We should already be able to do this, but this is more of a question of
    ///   how/whether to reflect this in the diagram.  Also, it raises the issue
    ///   of whetehr we should be potentially propagating more of `ShoddyType`
    ///   directly.
    ///
    /// TODO: Move to returning an explicit new return type in the option, which
    /// may or may not make sense to be wrapped in a vec.  Visually:
    /// - For maps the key and value may way to be on separate rows to avoid
    ///   arrow crossings.  The type name should be implicit in the target,
    ///   though.
    /// - Potential badges:
    ///   - Atom for atomic: U+269B \u269B
    ///   -
    ///
    /// Definitely real issues from llvm:
    /// - `llvm::MachineInstrBundleIterator::operator->` we get confused:
    ///   "Saw '>' when not in an argument"
    ///
    /// XXX Previously Pending issues that maybe I fixed some:
    /// - closing state hates commas and then the new type:
    ///   `AutoTArray<RefPtr<nsFrameSelection>, 1>`
    /// - `NotNull<nsCOMPtr<mozIStorageConnection> >` getting id clobber
    ///   `prev_id="nsCOMPtr" new_id=""`
    /// - `nsTArray<Accessible *>` the space trips us up.
    /// - Also maybe:
    ///   `Got an identifier when already had an identifier! type_str="Maybe<BufferPointer<BaselineFrame> >" prev_id="BufferPointer" new_id=""`
    /// - Also for arrays, seems like we should propagate that.
    ///   - `Vector<UniquePtr<ProfiledThreadData> >` is an interesting case of that.
    ///   - `const std::vector<HashMgr *> &`
    ///   - `Vector<char *>`
    ///   - `AutoTArray` in addition to `nsTArray`, `mozilla::Array`, nsTObserverArray
    /// - Also sets like HashSet
    /// - Native arrays?
    ///   - `"UniquePtr<char[]>"`
    ///   - `UniquePtr<nscoord[]>`
    /// - maybe just bail on functions because of complex signatures, ex tame:
    ///   `std::queue<std::function<void (void)> >`
    /// - Maybe just bail on unions?  ex:
    ///   `union AllocInfo::(anonymous at /builds/worker/checkouts/gecko/memory/build/mozjemalloc.cpp:3508:3)`
    /// - Similar with enums:
    ///   `enum (unnamed enum at /builds/worker/checkouts/gecko/xpcom/tests/gtest/TestMultiplexInputStream.cpp:503:3)`
    /// - Maybe need to "Evaluate" only on first arg for cases like
    ///   `UniquePtr<Utf8Unit[], JS::FreePolicy>` where right now we only call on the FreePolicy.
    ///
    /// Other domain situations:
    /// - `Rooted<AbstractGeneratorObject *>`
    /// - `Atomic<_Bool>`
    /// - DataMutex, StaticDataMutex
    ///   - So for these I think it makes sense for this to be a bool that gets mapped to an atomic emoji.
    /// - Maps: `nsTHashtable<CategoryLeaf>`, std::map
    ///   - For std::map need to reference key and value types!
    ///
    /// Complex scenarios:
    /// - `HashSet<gc::Cell *, DefaultHasher<gc::Cell *>, SystemAllocPolicy>` hates the closing state
    pub fn maybe_parse_type_as_pointer(&self, type_str: &str) -> Vec<(OntologyPointerKind, Ustr)> {
        let mut c = type_str.chars();
        let mut state = TypeParseState::Typish;
        let mut type_stack: Vec<ShoddyType> = vec![];
        let mut cur_type = ShoddyType::default();
        let mut token = String::new();

        let mut results: Vec<(OntologyPointerKind, Ustr)> = vec![];

        loop {
            let next_c = c.next();

            match (state, next_c) {
                (TypeParseState::Typish, None) => break,
                // Whitespace can happen in a few cases:
                // - After "const", so token.len() > 0
                // - After a legit token just before a "*".
                // - After a ",", so token.len() == 0
                // - After a ">", but we handle that via the `Closing` state.
                (TypeParseState::Typish, Some(' ')) => {
                    if token.len() > 0 {
                        if token.as_str() == "const" {
                            cur_type.is_const = true;
                        } else if token.as_str() == "union" {
                            // we can't do anything useful with unions.
                            return results;
                        } else if token.as_str() == "enum" {
                            // we can't do anything useful with enums.
                            return results;
                        } else if token.as_str() == "class" || token.as_str() == "struct" {
                            cur_type.is_tag = true;
                        } else if token.chars().all(char::is_numeric) {
                            // If our current token is just numeric then we're quite
                            // possibly looking at something like
                            // "1 << sizeof(AnonymousContentKey) * 8" as a template
                            // arg.  This is a complex case that I think really
                            // emphasizes we should just move to having clang give
                            // us a structured representation of the types and stop
                            // fooling around.  So we're just going to early return
                            // in this case rather than go down a shoddy parsing
                            // rabbit hole.
                            return results;
                        } else {
                            if cur_type.identifier.len() > 0 {
                                warn!(
                                    type_str,
                                    prev_id = cur_type.identifier,
                                    new_id = token,
                                    "Got an identifier when already had an identifier!"
                                );
                            }
                            cur_type.identifier = token;
                        }
                        token = String::new();
                    }
                    // otherwise this is probably after a comma.
                }
                (TypeParseState::Typish, Some('*')) => {
                    cur_type.is_pointer = true;
                    token = String::new();
                }
                (TypeParseState::Typish, Some('&')) => {
                    cur_type.is_ref = true;
                }
                (TypeParseState::Typish, Some('<')) => {
                    if cur_type.identifier.len() > 0 {
                        warn!(
                            type_str,
                            prev_id = cur_type.identifier,
                            new_id = token,
                            "Got an identifier when already had an identifier!"
                        );
                    }
                    cur_type.identifier = token;
                    token = String::new();

                    type_stack.push(cur_type);
                    cur_type = ShoddyType::default();
                }
                (TypeParseState::Typish, Some(',')) => {
                    if cur_type.identifier.len() > 0 {
                        warn!(
                            type_str,
                            prev_id = cur_type.identifier,
                            new_id = token,
                            "Got an identifier when already had an identifier!"
                        );
                    }
                    cur_type.identifier = token;
                    token = String::new();

                    // Evaluate the types now that cur_type is updated.
                    //
                    // TODO: Consider unifying with the `>` closing a bit more.
                    // Right now this is just to help mark the nothing type,
                    // and we don't really need to process pointers here because
                    // those will have a > and then be in closing and then see a
                    // ',', but we can do better or at least add more comments.
                    let parent_name = ustr(&cur_type.identifier);
                    match self.types.get(&parent_name) {
                        Some(OntologyType::Nothing) => {
                            cur_type.is_nothing = true;
                        }
                        _ => {}
                    }

                    if let Some(container_type) = type_stack.last_mut() {
                        container_type.args.push(cur_type);
                        cur_type = ShoddyType::default();
                    } else {
                        warn!(type_str, "Hit comma with no parent type!");
                        return results;
                    }
                }
                (TypeParseState::Typish, Some('>')) | (TypeParseState::Closing, Some('>')) => {
                    // In the closing state we don't process the token.
                    if state == TypeParseState::Typish {
                        if token.len() > 0 {
                            if cur_type.identifier.len() > 0 {
                                warn!(
                                    type_str,
                                    prev_id = cur_type.identifier,
                                    new_id = token,
                                    "Got an identifier when already had an identifier!"
                                );
                            }
                            cur_type.identifier = token;
                            token = String::new();
                        }
                    }

                    // A type is being closed out, the cur_type goes in the parent,
                    // and the parent becomes the new cur_type.
                    let done_type = cur_type;
                    cur_type = match type_stack.pop() {
                        Some(t) => t,
                        None => {
                            warn!(type_str, "Unpaired '>' encountered!");
                            return results;
                        }
                    };
                    cur_type.args.push(done_type);

                    // Evaluate the types now that cur_type is updated.
                    let parent_name = ustr(&cur_type.identifier);
                    match self.types.get(&parent_name) {
                        Some(OntologyType::Pointer(ptr)) => {
                            if let Some(arg_type) = cur_type.args.get(ptr.arg_index as usize) {
                                let pointee_name = ustr(&arg_type.identifier);
                                info!(
                                    type_str,
                                    parent_name = cur_type.identifier,
                                    pointee_name = pointee_name.as_str(),
                                    "evaluating"
                                );

                                if arg_type.is_tag
                                    && self.types.get(&pointee_name)
                                        != Some(&OntologyType::Value)
                                {
                                    results.push((ptr.kind.clone(), pointee_name));
                                }
                                cur_type.consumed = true;
                            }
                        }
                        Some(OntologyType::Variant) => {
                            // Push all the non-nothing types that weren't already pushed.
                            for arg_type in &cur_type.args {
                                if arg_type.consumed || arg_type.is_nothing {
                                    continue;
                                }
                                if arg_type.is_pointer {
                                    results.push((
                                        OntologyPointerKind::Raw,
                                        ustr(&arg_type.identifier),
                                    ));
                                } else if arg_type.is_ref {
                                    results.push((
                                        OntologyPointerKind::Ref,
                                        ustr(&arg_type.identifier),
                                    ));
                                } else if arg_type.is_tag {
                                    if let Some(OntologyType::Value) =
                                        self.types.get(&ustr(&arg_type.identifier))
                                    {
                                        // If the type is a value type, like nsTString, fall through to None.
                                    } else {
                                        results.push((
                                            OntologyPointerKind::Contains,
                                            ustr(&arg_type.identifier),
                                        ));
                                    }
                                }
                            }
                        }
                        Some(OntologyType::Nothing) => {
                            cur_type.is_nothing = true;
                        }
                        _ => {}
                    }

                    state = TypeParseState::Closing;
                }
                (TypeParseState::Typish, Some(c)) => {
                    token.push(c);
                }

                (TypeParseState::Closing, None) => {
                    assert_eq!(type_stack.len(), 0, "Should have closed all types.");
                    break;
                }
                (TypeParseState::Closing, Some(' ')) => {
                    // Whitespace doesn't mattern when closing.
                }
                (TypeParseState::Closing, Some(',')) => {
                    if let Some(container_type) = type_stack.last_mut() {
                        container_type.args.push(cur_type);
                        cur_type = ShoddyType::default();
                    } else {
                        warn!(type_str, "Hit comma with no parent type!");
                        return results;
                    }
                    // We're no longer closing.
                    state = TypeParseState::Typish;
                }
                (TypeParseState::Closing, Some('*')) => {
                    cur_type.is_pointer = true;
                }
                (TypeParseState::Closing, Some('&')) => {
                    cur_type.is_ref = true;
                }
                (TypeParseState::Closing, Some(c)) => {
                    info!(type_str, "Unexpected character in closing state: '{}'", c);
                }
            }
        }

        if token.len() > 0 {
            cur_type.identifier = token;
        }

        if results.is_empty() && !cur_type.consumed {
            if cur_type.is_pointer {
                info!(
                    type_str,
                    type_name = cur_type.identifier,
                    "fallback to pointer on exit"
                );
                results.push((OntologyPointerKind::Raw, ustr(&cur_type.identifier)));
            } else if cur_type.is_ref {
                info!(
                    type_str,
                    type_name = cur_type.identifier,
                    "fallback to ref on exit"
                );
                results.push((OntologyPointerKind::Ref, ustr(&cur_type.identifier)));
            } else if cur_type.is_tag {
                if let Some(OntologyType::Value) = self.types.get(&ustr(&cur_type.identifier)) {
                    // If the type is a value type, like nsTString, fall through to None.
                } else {
                    results.push((OntologyPointerKind::Contains, ustr(&cur_type.identifier)));
                }
            }
        }

        results
    }
}

#[test]
fn test_type_parser() {
    let test_config = r#"
[types."nsCOMPtr".pointer]
kind = "strong"

# explicitly not in the mozilla namespace
[types."RefPtr".pointer]
kind = "strong"

[types."mozilla::UniquePtr".pointer]
kind = "unique"

[types."UniquePtr".pointer]
kind = "unique"

[types."mozilla::WeakPtr".pointer]
kind = "weak"

[types."WeakPtr".pointer]
kind = "weak"

[types."nsClassHashtable".pointer]
kind = "unique"
arg_index = 1

[types."nsTString".value]

# ### Variant Types ###
[types."mozilla::Variant".variant]

# ### Sentinel Nothing Types ###
[types."mozilla::Nothing".nothing]

[types."mozilla::Maybe".pointer]
kind = "contains"
"#;
    let ingestion = OntologyMappingIngestion::new(test_config).unwrap();
    let c = &ingestion.config;

    assert_eq!(c.maybe_parse_type_as_pointer("_Bool"), vec![]);

    // Note that some of these real-world examples pre-date our change to use the
    // canonical type which gets us fully qualified namespaces, so these won't
    // match reality.
    assert_eq!(
        c.maybe_parse_type_as_pointer("class RefPtr<class outer::inner::Actual>"),
        vec![(OntologyPointerKind::Strong, ustr("outer::inner::Actual"))]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("UniquePtr<class Poodle, JS::FreePolicy>"),
        vec![(OntologyPointerKind::Unique, ustr("Poodle"))]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("AutoTArray<RefPtr<class nsFrameSelection>, 1>"),
        vec![(OntologyPointerKind::Strong, ustr("nsFrameSelection"))]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("NotNull<nsCOMPtr<class mozIStorageConnection> >"),
        vec![(OntologyPointerKind::Strong, ustr("mozIStorageConnection"))]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("NotNull<nsCOMPtr<class mozIStorageConnection> >"),
        vec![(OntologyPointerKind::Strong, ustr("mozIStorageConnection"))]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("union AllocInfo::(anonymous at /builds/worker/checkouts/gecko/memory/build/mozjemalloc.cpp:3508:3)"),
        vec![]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer(
            "class nsClassHashtable<class nsCStringHashKey, class RegistrationDataPerPrincipal>"
        ),
        vec![(
            OntologyPointerKind::Unique,
            ustr("RegistrationDataPerPrincipal")
        )]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("HashSet<RefPtr<class ServiceWorkerRegistrationInfo>, class PointerHasher<ServiceWorkerRegistrationInfo*>>"),
        vec![(OntologyPointerKind::Strong, ustr("ServiceWorkerRegistrationInfo"))]
    );

    // const struct mozilla::dom::locks::LockRequest
    assert_eq!(
        c.maybe_parse_type_as_pointer("const struct mozilla::dom::locks::LockRequest"),
        vec![(
            OntologyPointerKind::Contains,
            ustr("mozilla::dom::locks::LockRequest")
        )]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("class nsTString<char16_t>"),
        vec![]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("class mozilla::Variant<struct mozilla::Nothing, class RefPtr<class nsPIDOMWindowInner>, class nsCOMPtr<class nsIDocShell>, class mozilla::dom::WorkerPrivate *>"),
        vec![
            (OntologyPointerKind::Strong, ustr("nsPIDOMWindowInner")),
            (OntologyPointerKind::Strong, ustr("nsIDocShell")),
            (OntologyPointerKind::Raw, ustr("mozilla::dom::WorkerPrivate"))
        ]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("class mozilla::Maybe<class nsTString<char16_t> >"),
        vec![]
    );

    assert_eq!(
        c.maybe_parse_type_as_pointer("Array<std::pair<uint8_t, uint8_t>, 1 << sizeof(AnonymousContentKey) * 8>"),
        vec![]
    );
}
