;; Groovy's (Gradle's) blocks, by the methods they're closures of (ex:
;; `android { defaultConfig { ... } }`), functions, methods, and classes.

(((method_invocation name: (identifier) @name body: (closure)) @container)
  (#set! structure.kind "block"))

(((function_definition name: (identifier) @name) @container)
  (#set! structure.kind "method"))

(((method_declaration name: (identifier) @name) @container)
  (#set! structure.kind "method"))

(((class_declaration name: (identifier) @name) @container)
  (#set! structure.kind "class"))
