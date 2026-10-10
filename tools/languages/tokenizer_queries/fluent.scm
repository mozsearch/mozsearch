;; Fluent containers, for the history tokenizer (see cst_tokenizer.rs), with
;; searchfox's tree-sitter-fluent: messages, terms (whose identifiers include
;; their "-"), and their attributes (ex: `menu-open::accesskey`).

(((message id: (identifier) @name) @container)
  (#set! structure.kind "message"))

(((term id: (term_identifier) @name) @container)
  (#set! structure.kind "term"))

(((attribute id: (identifier) @name) @container)
  (#set! structure.kind "attribute"))
