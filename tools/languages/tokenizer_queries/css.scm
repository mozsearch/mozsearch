;; CSS's containers (SCSS's too; see scss.scm): rule sets by their selectors
;; (ex: `#nav-bar > toolbarbutton:hover`), keyframes by their names, and
;; `@media` and `@supports` blocks by their (first) conditions.

(((rule_set (selectors) @name) @container)
  (#set! structure.kind "rule"))

(((keyframes_statement (keyframes_name) @name) @container)
  (#set! structure.kind "keyframes"))

(((media_statement . (_) @name) @container)
  (#set! structure.kind "media"))

(((supports_statement . (_) @name) @container)
  (#set! structure.kind "supports"))
