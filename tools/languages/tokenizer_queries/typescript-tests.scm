;; Tests' names, for files whose `searchfox-contexts` history attribute is
;; `tests` (see cst_tokenizer's `ContextMode::Tests`), added to typescript.scm's
;; containers: testharness's `test(..., "description")` and its kin, named by
;; their descriptions' constant parts (a template string's text before its
;; first substitution).  (Mochitest's and xpcshell's `add_task(async function
;; test_foo() {...})` and `add_test(function test_foo() {...})` are already
;; named, by their function expressions.)

(((call_expression
    function: (identifier) @_harness
    arguments: (arguments (string (string_fragment) @name))) @container)
  (#match? @_harness "^(test|async_test|promise_test|subsetTest|promise_setup|idl_test|spawn_task)$")
  (#set! structure.kind "test"))

(((call_expression
    function: (identifier) @_harness
    arguments: (arguments (template_string . (string_fragment) @name))) @container)
  (#match? @_harness "^(test|async_test|promise_test|subsetTest|promise_setup|idl_test|spawn_task)$")
  (#set! structure.kind "test"))
