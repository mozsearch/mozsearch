;; derived from both of the following plus our scip-indexer.rs decisions:
;; - https://github.com/tree-sitter/tree-sitter-javascript/blob/master/queries/tags.scm
;; - https://github.com/tree-sitter/tree-sitter-typescript/blob/master/queries/tags.scm
;;
;; We retain the tag annotations like "definition" that we don't care about since
;; it might aid in diagnostics.
;;
;; double-comments like this are distinct from the original source.

;; JS

(method_definition
    name: (property_identifier) @name) @container

[
  (class
    name: (_) @name)
  (class_declaration
    name: (_) @name)
] @container

[
  (function
    name: (identifier) @name)
  (function_declaration
    name: (identifier) @name)
  (generator_function
    name: (identifier) @name)
  (generator_function_declaration
    name: (identifier) @name)
] @container

;; I'm assuming the lexical_declaration can have multiple children because of
;; JS allowing multiple decls via use of commas, so we're leaving the container
;; on the specific declarator.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function)]) @container)


(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function)]) @container)

(assignment_expression
  left: [
    (identifier) @name
    (member_expression
      property: (property_identifier) @name)
  ]
  right: [(arrow_function) (function)]
) @container

(pair
  key: (property_identifier) @name
  value: [(arrow_function) (function)]) @container

(export_statement value: (assignment_expression left: (identifier) @name right: ([
 (number)
 (string)
 (identifier)
 (undefined)
 (null)
 (new_expression)
 (binary_expression)
 (call_expression)
]))) @container

;; TS

(function_signature
  name: (identifier) @name) @container

(method_signature
  name: (property_identifier) @name) @container

(abstract_method_signature
  name: (property_identifier) @name) @container

(abstract_class_declaration
  name: (type_identifier) @name) @container

(module
  name: (identifier) @name) @container

(interface_declaration
  name: (type_identifier) @name) @container
