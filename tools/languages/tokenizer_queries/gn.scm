;; GN's targets and templates, by their names: a call's string argument, ex:
;; `executable("foo")`, names the block after it (tree-sitter-gn parses the
;; block as the call's sibling, not its child).

(((_
    (primary_expression
      (call_expression
        function: (identifier)
        (primary_expression (string (string_content) @name))))
    .
    (primary_expression (block) @container)))
  (#set! structure.kind "target"))
