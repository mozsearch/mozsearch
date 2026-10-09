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

;; Classes and structs can be named by qualified names (ex: an out-of-line
;; definition of a nested class, `class WorkerPrivate::EventTarget final`) and
;; template specializations (`struct Hash<Foo>`).
(((struct_specifier
  name: [(type_identifier) (qualified_identifier) (template_type)] @name
  body:(_)) @container)
  (#set! structure.kind "struct"))

(((declaration
  type: (union_specifier
    name: (type_identifier) @name)) @container)
  (#set! structure.kind "union"))

;; (Unions defined elsewhere, ex: as fields' types, in `union Type {...} mType;`.)
(((union_specifier
  name: (type_identifier) @name
  body: (_)) @container)
  (#set! structure.kind "union"))

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
;;
;; The name has to be one of those (or a destructor, operator, template
;; specialization, or a function pointer's `(*name)`), since tree-sitter-cpp
;; takes calls of statement macros with a lambda argument, ex:
;; `QM_TRY_UNWRAP(auto x, ([&]() -> Result<...> {...}()));` inside a function,
;; for function definitions whose name is a `function_declarator` or
;; `array_declarator` for the macro call.
;;
;; Functions returning pointers or references have their `function_declarator`
;; inside a `pointer_declarator` or `reference_declarator` (or two, for
;; `char** Foo()`).
(((function_definition
  declarator: [
    (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name)
    (pointer_declarator declarator: (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name))
    (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name)))
    (reference_declarator (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name))
  ]) @container)
  (#set! structure.kind "method"))

;; Conversion operators (`operator bool() const`), which have no name node: the
;; `operator_cast` (or the qualified name ending in one) is their name, without
;; its parameters and qualifiers (see `name_text` in cst_tokenizer.rs), ex:
;; `operator bool`, `Foo::operator const char*`.
(((function_definition
  declarator: [
    (operator_cast)
    (qualified_identifier name: (operator_cast))
    (qualified_identifier name: (qualified_identifier name: (operator_cast)))
  ] @name) @container)
  (#set! structure.kind "method"))

;; (In classes, they're declarations, not field declarations.)
(((field_declaration_list
  (declaration
    declarator: (operator_cast) @name) @container))
  (#set! structure.kind "field"))

;; Defaulted and deleted functions defined outside of their classes, whose
;; return types have `&` or `*` (ex: `Foo& Foo::operator=(Foo&&) = default;`),
;; tree-sitter-cpp takes for declarations initialized with `default` or
;; `delete` (and the others for function definitions).
(((declaration
  declarator: (init_declarator
    declarator: [
      (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
      ] @name)
      (pointer_declarator declarator: (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
      ] @name))
      (reference_declarator (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
      ] @name))
    ]
    value: (identifier) @value)) @container
  (#any-of? @value "default" "delete"))
  (#set! structure.kind "method"))

(((field_declaration
  declarator: [
    (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name)
    (pointer_declarator declarator: (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name))
    (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name)))
    (reference_declarator (function_declarator declarator: [
      (identifier)
      (field_identifier)
      (qualified_identifier)
      (destructor_name)
      (operator_name)
      (template_function)
      (parenthesized_declarator)
    ] @name))
  ]) @container)
  (#set! structure.kind "field"))

;; Field definitions for members will just have a field_identifier (versus the
;; `function_declarator` above.  This also provides containment for any
;; `default_value`.
(((field_declaration
  declarator: (field_identifier) @name) @container)
  (#set! structure.kind "field"))

;; And fields whose declarators are pointers, references, or arrays (ex: `Foo*
;; mFoo;`, `const char* const mName;`, `Foo& mRef;`, `char mBuf[N];`), which
;; are their own contexts like the others: a field's changes are its type's,
;; annotations' (ex: `MOZ_GUARDED_BY(mMutex)`), and comments', not its class's
;; other fields'.
(((field_declaration
  declarator: [
    (pointer_declarator declarator: (field_identifier) @name)
    (pointer_declarator declarator: (pointer_declarator declarator: (field_identifier) @name))
    (pointer_declarator declarator: (array_declarator declarator: (field_identifier) @name))
    (reference_declarator (field_identifier) @name)
    (array_declarator declarator: (field_identifier) @name)
    (array_declarator declarator: (array_declarator declarator: (field_identifier) @name))
  ]) @container)
  (#set! structure.kind "field"))

;; Note that we can end up with multiple declarators as in the example
;; `typedef struct {int a; int b;} S, *pS;` from
;; https://en.cppreference.com/w/cpp/language/typedef but if we just favor the
;; first thing matching the given root container node, that should be fine.
;; (Also, this should be a rare idiom hopefully!)
(((type_definition
  declarator: (type_identifier) @name) @container)
  (#set! structure.kind "typedef"))

(((enum_specifier
  name: (type_identifier) @name
  body: (_)) @container)
  (#set! structure.kind "enum"))

;; (Classes, like enums, structs, and unions, are containers where they're
;; defined, not where they're mentioned, ex: in forward declarations, `friend
;; class Bar;`, and `class Foo* aFoo`.)
(((class_specifier
  name: [(type_identifier) (qualified_identifier) (template_type)] @name
  body: (_)) @container)
  (#set! structure.kind "class"))

;; Explicit instantiations of class templates (ex: `template class
;; DecoderTemplate<VideoDecoderTraits>;`), which are definitions, but not
;; explicit instantiation declarations (`extern template class ...;`).
(((template_instantiation
  .
  "template"
  type: [
    (class_specifier
      name: [(type_identifier) (qualified_identifier) (template_type)] @name)
    (struct_specifier
      name: [(type_identifier) (qualified_identifier) (template_type)] @name)
  ]) @container)
  (#set! structure.kind "class"))

;; For `namespace foo {}` the name is an `identifier`, but for
;; `namespace foo::bar` the name is an `namespace_definition_name` which has
;; multiple `identifier` children.
(((namespace_definition
  name: (_) @name) @container)
  (#set! structure.kind "namespace"))
