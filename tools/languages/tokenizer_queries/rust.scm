;; derived from the following plus our scip-indexer.rs decisions:
;; - https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm
;;
;; We retain the tag annotations like "definition" that we don't care about since
;; it might aid in diagnostics.
;;
;; double-comments like this are distinct from the original source.

; ADT definitions

(struct_item
    name: (type_identifier) @name) @container

(enum_item
    name: (type_identifier) @name) @container

(union_item
    name: (type_identifier) @name) @container

;; we skip type aliases

; function definitions

(function_item
    name: (identifier) @name) @container

; trait definitions
(trait_item
    name: (type_identifier) @name) @container

; module definitions
(mod_item
    name: (identifier) @name) @container


;; implementations; we're following our decision in scip-indexer.rs to only care
;; about the type and not the trait, we diverge here.

(impl_item
    type: (type_identifier) @name) @container
