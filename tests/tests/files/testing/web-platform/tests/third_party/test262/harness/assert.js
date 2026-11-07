/*---
description: A stub of Test262's assertions.
defines: [assert]
---*/
function assert(mustBeTrue, message) {
  if (mustBeTrue !== true) {
    throw new Test262Error(message);
  }
}

assert.sameValue = function (actual, expected, message) {
  assert(actual === expected, message);
};
