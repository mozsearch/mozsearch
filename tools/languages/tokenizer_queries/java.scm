;; Java containers, for the history tokenizer (see cst_tokenizer.rs), like
;; scip-indexer.rs's JAVA_NESTING, plus enums, records, constructors, and
;; fields.  Packages aren't containers, since they're statements rather than
;; blocks.

(([
  (class_declaration name: (identifier) @name)
  (interface_declaration name: (identifier) @name)
  (annotation_type_declaration name: (identifier) @name)
  (record_declaration name: (identifier) @name)
] @container)
  (#set! structure.kind "class"))

(((enum_declaration
    name: (identifier) @name) @container)
  (#set! structure.kind "enum"))

(([
  (method_declaration name: (identifier) @name)
  (constructor_declaration name: (identifier) @name)
  (compact_constructor_declaration name: (identifier) @name)
] @container)
  (#set! structure.kind "method"))

;; (With more than one declarator, ex: `int mX, mY;`, the first names it.)
(((field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @container)
  (#set! structure.kind "field"))
