;; derived from the following plus our scip-indexer.rs decisions:
;; - https://github.com/tree-sitter/tree-sitter-python/blob/master/queries/tags.scm

(class_definition
  name: (identifier) @name) @container

(function_definition
  name: (identifier) @name) @container
