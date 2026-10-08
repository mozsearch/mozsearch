;; XPIDL containers, for the history tokenizer (see cst_tokenizer.rs), with
;; tree-sitter-xpidl: interfaces (not forward declarations) and their members.

(((interface_definition
    name: (identifier) @name
    body: (_)) @container)
  (#set! structure.kind "class"))

(((method
    name: (identifier) @name) @container)
  (#set! structure.kind "method"))

(([
  (attribute_declaration name: (identifier) @name)
  (constant name: (identifier) @name)
] @container)
  (#set! structure.kind "field"))

(((cenum
    name: (identifier) @name) @container)
  (#set! structure.kind "enum"))

(([
  (typedef name: (identifier) @name)
  (native name: (identifier) @name)
] @container)
  (#set! structure.kind "typedef"))
