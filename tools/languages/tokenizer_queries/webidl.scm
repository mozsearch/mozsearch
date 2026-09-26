;; Structure for tree-sitter-webidl (https://github.com/padenot/tree-sitter-webidl).
;;
;; Partial interfaces/mixins/dictionaries/namespaces produce the same pretty
;; identifiers as their primary definitions, so their members share history in
;; the symdex.  Operations can be overloaded, which results in duplicate pretty
;; identifiers, which is fine.

(((interface name: (identifier) @name) @container)
  (#set! structure.kind "class"))

(((partial_interface name: (identifier) @name) @container)
  (#set! structure.kind "class"))

(((mixin name: (identifier) @name) @container)
  (#set! structure.kind "class"))

(((partial_mixin name: (identifier) @name) @container)
  (#set! structure.kind "class"))

(((callback_interface name: (identifier) @name) @container)
  (#set! structure.kind "class"))

(((namespace name: (identifier) @name) @container)
  (#set! structure.kind "namespace"))

(((partial_namespace name: (identifier) @name) @container)
  (#set! structure.kind "namespace"))

(((dictionary name: (identifier) @name) @container)
  (#set! structure.kind "struct"))

(((partial_dictionary name: (identifier) @name) @container)
  (#set! structure.kind "struct"))

(((enum name: (identifier) @name) @container)
  (#set! structure.kind "enum"))

(((typedef name: (identifier) @name) @container)
  (#set! structure.kind "typedef"))

(((callback name: (identifier) @name) @container)
  (#set! structure.kind "function"))

(((callback_constructor name: (identifier) @name) @container)
  (#set! structure.kind "function"))

;; Members

(((operation name: (identifier) @name) @container)
  (#set! structure.kind "method"))

;; Constructors don't have a name, so we use the `constructor` keyword.
(((constructor "constructor" @name) @container)
  (#set! structure.kind "method"))

(((attribute name: (identifier) @name) @container)
  (#set! structure.kind "field"))

(((const name: (identifier) @name) @container)
  (#set! structure.kind "field"))

;; Dictionary members.
(((field name: (identifier) @name) @container)
  (#set! structure.kind "field"))
