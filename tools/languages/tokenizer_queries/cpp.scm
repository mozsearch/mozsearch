;; derived from https://github.com/tree-sitter/tree-sitter-cpp/pull/189
;; (tree-sitter-cpp currently otherwise lacks a tags.scm)
;;
;; expanded to also include the parent node, like we want the whole
;; `function_definition`, not just its `declarator: function_declarator` so that
;; we can also get the `body: compound_statement`.
;;
;; We also currently don't want to split out the scope from the identifier.  The
;; trade-off is that we get simplicity since we only need to deal with a single
;; captured node in our code, but with the C++ "::" delimiter baked in to the
;; string.

(struct_specifier
  name: (type_identifier) @name
  body:(_)) @container

(declaration
  type: (union_specifier
    name: (type_identifier) @name)) @container

;; We explicitly don't provide a type for the "@name"; this lets us cover all of
;; - `identifier`: top-level function (not part of a class/struct)
;; - `field_identifier`: method decl/inline def (part of a class/struct)
;; - `qualified_identifier`: method def outside of the class def.  Common case
;;   has a `scope: namespace_identifier` and `name: identifier`.  As noted
;;   above, we like just using the full qualified_identifier here.
;;
;; Note that for template functions, the `function_definition` will be the
;; child of a `template_declaration` which we currently don't handle, which
;; means the template won't get marked with the function as context.  An
;; option might be to just have a separate match on `(template_declaration
;; parameters: (template_parameter_list) @name) @container`.  For
;; `template<T, X> void foo(...)` the name is then `<T, X>` which is weird but
;; workable.
;;
;; Also, `function_definition` is for inline definitions, whereas
;; `field_declaration` is for when it's just a decl and the def is elsewhere.
(function_definition
  declarator: (function_declarator
    declarator: (_) @name)) @container

(field_declaration
  declarator: (function_declarator
    declarator: (_) @name)) @container

;; Field definitions for members will just have a field_identifier (versus the
;; `function_declarator` above.  This also provides containment for any
;; `default_value`.
(field_declaration
  declarator: (field_identifier) @name) @container

;; Note that we can end up with multiple declarators as in the example
;; `typedef struct {int a; int b;} S, *pS;` from
;; https://en.cppreference.com/w/cpp/language/typedef but if we just favor the
;; first thing matching the given root container node, that should be fine.
;; (Also, this should be a rare idiom hopefully!)
(type_definition
  declarator: (type_identifier) @name) @container

(enum_specifier
  name: (type_identifier) @name) @container

(class_specifier
  name: (type_identifier) @name) @container

;; For `namespace foo {}` the name is an `identifier`, but for
;; `namespace foo::bar` the name is an `namespace_definition_name` which has
;; multiple `identifier` children.
(namespace_definition
  name: (_) @name) @container
