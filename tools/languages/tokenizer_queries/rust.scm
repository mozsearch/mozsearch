;; derived from the following plus our scip-indexer.rs decisions:
;; - https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm
;;
;; We retain the tag annotations like "definition" that we don't care about since
;; it might aid in diagnostics.
;;
;; double-comments like this are distinct from the original source.

; ADT definitions

(((struct_item
    name: (type_identifier) @name) @container)
    (#set! structure.kind "struct"))

(((enum_item
    name: (type_identifier) @name) @container)
    (#set! structure.kind "enum"))

(((union_item
    name: (type_identifier) @name) @container)
    (#set! structure.kind "union"))

;; Named fields of structs, unions, and enums' struct-like variants (not tuple
;; structs'), so that their comments and attributes are theirs, not their
;; types', like C++'s fields.
(((field_declaration
    name: (field_identifier) @name) @container)
    (#set! structure.kind "field"))

;; we skip type aliases

; function definitions
;; uh, for "structure.kind" I'm dubiously mapping to clases/methods for
;; expediency.

(((function_item
    name: (identifier) @name) @container)
    (#set! structure.kind "method"))

;; Functions without bodies, in `extern` blocks and traits.
(((function_signature_item
    name: (identifier) @name) @container)
    (#set! structure.kind "method"))

; trait definitions
(((trait_item
    name: (type_identifier) @name) @container)
    (#set! structure.kind "class"))


; module definitions
(((mod_item
    name: (identifier) @name) @container)
    (#set! structure.kind "namespace"))

;; implementations; we're following our decision in scip-indexer.rs to only care
;; about the type and not the trait, we diverge here.

(((impl_item
    type: (type_identifier) @name) @container)
    (#set! structure.kind "class"))
