use serde::{Deserialize, Serialize};
use ustr::{ustr, UstrMap, Ustr};

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
    pub label_field_uses: Option<OntologyLabelFieldUses>,
}

#[derive(Deserialize)]
pub struct OntologyLabelFieldUses {
    #[serde(default)]
    pub labels: Vec<OntologyLabelRule>,
}

#[derive(Deserialize)]
pub struct OntologyLabelRule {
    pub context_sym_suffix: Ustr,
    pub label: Ustr,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OntologyType {
    Pointer(OntologyTypePointer)
}

#[derive(Deserialize)]
pub struct OntologyTypePointer {
    pub kind: OntologyPointerKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OntologyPointerKind {
    Strong,
    Unique,
    Weak,
    Raw
}

pub struct OntologyMappingIngestion {
    pub config: OntologyMappingConfig,
}

impl OntologyMappingIngestion {
    pub fn new(config_str: &str) -> Result<Self, String> {
        let config: OntologyMappingConfig =
            toml::from_str(config_str).map_err(|err| err.to_string())?;

        Ok(OntologyMappingIngestion {
            config,
        })
    }
}

#[derive(Clone, Copy, Debug)]
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
    identifier: String,
    args: Vec<ShoddyType>,
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
    pub fn maybe_parse_type_as_pointer(&self, type_str: &str) -> Option<(OntologyPointerKind, Ustr)> {
        let mut c = type_str.chars();
        let mut state = TypeParseState::Typish;
        let mut type_stack: Vec<ShoddyType> = vec![];
        let mut cur_type = ShoddyType::default();
        let mut token = String::new();

        let mut result: Option<(OntologyPointerKind, Ustr)> = None;

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
                        } else if token.as_str() == "class" {
                            // did I intentionally have us add markers here?
                        } else {
                            if cur_type.identifier.len() > 0 {
                                warn!(type_str, prev_id=cur_type.identifier, new_id=token, "Got an identifier when already had an identifier!");
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
                (TypeParseState::Typish, Some('<')) => {
                    if cur_type.identifier.len() > 0 {
                        warn!(type_str, prev_id=cur_type.identifier, new_id=token, "Got an identifier when already had an identifier!");
                    }
                    cur_type.identifier = token;
                    token = String::new();

                    type_stack.push(cur_type);
                    cur_type = ShoddyType::default();
                }
                (TypeParseState::Typish, Some(',')) => {
                    // We're done parsing the cur_type and should push it into
                    // our parent's type.
                    if cur_type.identifier.len() > 0 {
                        warn!(type_str, prev_id=cur_type.identifier, new_id=token, "Got an identifier when already had an identifier!");
                    }
                    cur_type.identifier = token;
                    token = String::new();

                    if let Some(container_type) = type_stack.last_mut() {
                        container_type.args.push(cur_type);
                        cur_type = ShoddyType::default();
                    } else {
                        warn!(type_str, "Hit comma with no parent type!");
                        return None;
                    }
                }
                (TypeParseState::Typish, Some('>')) |
                (TypeParseState::Closing, Some('>')) => {
                    if cur_type.identifier.len() > 0 {
                        warn!(type_str, prev_id=cur_type.identifier, new_id=token, "Got an identifier when already had an identifier!");
                    }
                    cur_type.identifier = token;
                    token = String::new();

                    // A type is being closed out, the cur_type goes in the parent,
                    // and the parent becomes the new cur_type.
                    let done_type = cur_type;
                    cur_type = match type_stack.pop() {
                        Some(t) => t,
                        None => {
                            warn!(type_str, "Unpaired '>' encountered!");
                            return None;
                        }
                    };

                    // Evaluate the types here.
                    let parent_name = ustr(&cur_type.identifier);
                    info!(type_str, parent_name = cur_type.identifier, pointee_name = done_type.identifier, "evaluating");
                    if let Some(OntologyType::Pointer(ptr)) = self.types.get(&parent_name) {
                        let pointee_name = ustr(&done_type.identifier);
                        if let Some(existing) = result {
                            warn!(type_str, "Clobbering existing result with pointee type: {}", existing.1);
                        }
                        result = Some((ptr.kind.clone(), pointee_name));
                    }

                    cur_type.args.push(done_type);

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
                (TypeParseState::Closing, Some(c)) => {
                    info!(type_str, "Unexpected character in closing state: '{}'", c);
                }
            }
        }

        if result.is_none() {
            if cur_type.is_pointer {
                return Some((OntologyPointerKind::Raw, ustr(&cur_type.identifier)));
            }
        }

        result
    }
}
