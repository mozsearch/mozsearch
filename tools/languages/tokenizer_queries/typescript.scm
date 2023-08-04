;; derived from both of the following plus our scip-indexer.rs decisions:
;; - https://github.com/tree-sitter/tree-sitter-javascript/blob/master/queries/tags.scm
;; - https://github.com/tree-sitter/tree-sitter-typescript/blob/master/queries/tags.scm
;;
;; We retain the tag annotations like "definition" that we don't care about since
;; it might aid in diagnostics.
;;
;; double-comments like this are distinct from the original source.

;; JS

(((method_definition
    name: (property_identifier) @name) @container)
  (#set! structure.kind "method"))

(([
  (class
    name: (_) @name)
  (class_declaration
    name: (_) @name)
] @container)
  (#set! structure.kind "class"))

(([
  (function
    name: (identifier) @name)
  (function_declaration
    name: (identifier) @name)
  (generator_function
    name: (identifier) @name)
  (generator_function_declaration
    name: (identifier) @name)
] @container)
  (#set! structure.kind "method"))

;; I'm assuming the lexical_declaration can have multiple children because of
;; JS allowing multiple decls via use of commas, so we're leaving the container
;; on the specific declarator.
((lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function)]) @container)
  (#set! structure.kind "lexdecl"))

((variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function)]) @container)
  (#set! structure.kind "lexdecl"))

(((assignment_expression
  left: [
    (identifier) @name
    (member_expression
      property: (property_identifier) @name)
  ]
  right: [(arrow_function) (function)]
) @container)
  (#set! structure.kind "lexdecl"))


(((pair
  key: (property_identifier) @name
  value: [(arrow_function) (function)]) @container)
  (#set! structure.kind "lexdecl"))

(((export_statement value: (assignment_expression left: (identifier) @name right: ([
 (number)
 (string)
 (identifier)
 (undefined)
 (null)
 (new_expression)
 (binary_expression)
 (call_expression)
]))) @container)
  (#set! structure.kind "lexdecl"))

;; TS

(((function_signature
  name: (identifier) @name) @container)
  (#set! structure.kind "function"))

(((method_signature
  name: (property_identifier) @name) @container)
  (#set! structure.kind "method"))

(((abstract_method_signature
  name: (property_identifier) @name) @container)
  (#set! structure.kind "method"))

(((abstract_class_declaration
  name: (type_identifier) @name) @container)
  (#set! structure.kind "class"))

(((module
  name: (identifier) @name) @container)
  (#set! structure.kind "namespace"))

(((interface_declaration
  name: (type_identifier) @name) @container)
  (#set! structure.kind "class"))
