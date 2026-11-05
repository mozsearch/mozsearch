// A JS shell test harness's helpers, as SpiderMonkey's tests' shell.js's:
// defined in a function it calls at once, and exposed as its global's
// properties.
(function (global) {
  var Error = global.Error;

  function assertSame(actual, expected) {
    if (actual !== expected) {
      throw new Error("not the same");
    }
  }
  global.assertSame = assertSame;

  global.assertAbout = function (actual, expected) {
    assertSame(Math.round(actual), Math.round(expected));
  };
})(this);
