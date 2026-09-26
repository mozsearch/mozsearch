;; Structure for tree-sitter-ipdl (https://github.com/padenot/tree-sitter-ipdl).
;;
;; The grammar doesn't use fields, but each of these declarations has exactly
;; one direct `identifier` child which is its name; everything else is nested
;; in other nodes (ex: `type_name`, `parameter_list`, `attribute_list`).
;;
;; Namespaces are containers, so pretty identifiers look like the C++ ones, ex:
;; `mozilla::dom::PFoo::Start`.

(((namespace_declaration (identifier) @name) @container)
  (#set! structure.kind "namespace"))

(((protocol_declaration (identifier) @name) @container)
  (#set! structure.kind "class"))

(((struct_declaration (identifier) @name) @container)
  (#set! structure.kind "struct"))

(((union_declaration (identifier) @name) @container)
  (#set! structure.kind "union"))

(((message_declaration (identifier) @name) @container)
  (#set! structure.kind "method"))

(((struct_field (identifier) @name) @container)
  (#set! structure.kind "field"))
