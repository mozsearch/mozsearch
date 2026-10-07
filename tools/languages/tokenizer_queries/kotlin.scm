;; Kotlin containers, for the history tokenizer (see cst_tokenizer.rs), like
;; scip-indexer.rs's KOTLIN_NESTING, plus objects and properties.
;; tree-sitter-kotlin-ng's `class_declaration`s include interfaces and enum
;; classes.  (Companion objects and secondary constructors, which may not have
;; names, aren't containers.)

(([
  (class_declaration name: (identifier) @name)
  (object_declaration name: (identifier) @name)
] @container)
  (#set! structure.kind "class"))

(((function_declaration
    name: (identifier) @name) @container)
  (#set! structure.kind "method"))

;; Properties of classes and files, but not functions' local variables.
((class_body
  (property_declaration
    (variable_declaration
      (identifier) @name)) @container)
  (#set! structure.kind "field"))

((source_file
  (property_declaration
    (variable_declaration
      (identifier) @name)) @container)
  (#set! structure.kind "field"))
