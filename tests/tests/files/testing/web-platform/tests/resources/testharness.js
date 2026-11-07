// A stub of web-platform-tests' testharness.js, whose API it defines in a
// function it calls at once, exposing it on its global (see the tests tree's
// js-environments.toml's `unwrap`).
(function (global_scope) {
  function test(func, name) {
    func();
  }

  function assert_equals(actual, expected, description) {
    if (actual !== expected) {
      throw new Error(description);
    }
  }

  function done() {}

  global_scope.test = test;
  global_scope.assert_equals = assert_equals;
  global_scope.done = done;
})(self);
