;; CMake's functions and macros, by their names (their commands' first
;; arguments).

(((function_def (function_command (argument_list . (argument) @name))) @container)
  (#set! structure.kind "function"))

(((macro_def (macro_command (argument_list . (argument) @name))) @container)
  (#set! structure.kind "macro"))
