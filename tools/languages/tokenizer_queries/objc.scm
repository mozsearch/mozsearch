;; Objective-C containers, for the history tokenizer (see cst_tokenizer.rs),
;; with tree-sitter-objc: classes' interfaces and implementations (named by
;; their first identifier, the class), protocols, methods (named by their
;; selectors' first parts, ex: `initWithFrame` for
;; `initWithFrame:geckoChild:`), and C's functions, structs, enums, and unions.

(([
  (class_interface . (identifier) @name)
  (class_implementation . (identifier) @name)
  (protocol_declaration . (identifier) @name)
] @container)
  (#set! structure.kind "class"))

(((method_definition
    (identifier) @name) @container)
  (#set! structure.kind "method"))

(((method_declaration
    (identifier) @name) @container)
  (#set! structure.kind "method"))

(((function_definition
    declarator: [
      (function_declarator declarator: (identifier) @name)
      (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))
    ]) @container)
  (#set! structure.kind "method"))

(((struct_specifier
    name: (type_identifier) @name
    body: (_)) @container)
  (#set! structure.kind "struct"))

(((enum_specifier
    name: (type_identifier) @name) @container)
  (#set! structure.kind "enum"))

;; (Like cpp.scm's, for fields' default values and consistency.)
(((field_declaration
    declarator: (field_identifier) @name) @container)
  (#set! structure.kind "field"))
