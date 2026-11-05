// A browser-chrome harness's test scope, as testing/mochitest/browser-test.js's:
// its tests run in a testScope, whose members they see as globals.
function testScope() {
  this.ok = function test_ok(condition, name) {};
}

testScope.prototype = {
  add_task(task) {},
};
