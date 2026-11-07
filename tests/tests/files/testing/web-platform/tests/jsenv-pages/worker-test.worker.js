// A `.worker.js` test, which its page starts as it is: it loads its harness
// itself.
importScripts("/resources/testharness.js");

test(() => {
  assert_equals(typeof postMessage, "function");
}, "A dedicated worker's");

done();
