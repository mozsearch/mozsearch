;; Makefiles' rules, by their targets, and `define`s, by their names.

(((rule (targets) @name) @container)
  (#set! structure.kind "rule"))

(((define_directive name: (_) @name) @container)
  (#set! structure.kind "define"))
