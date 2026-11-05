// Another xpcshell test after head_jsenv.js, whose call of its helper isn't
// one in test_shared.js's environment, which test_shared.js's diagrams leave
// out (see tools/src/js_environments/scope.rs).
function run_test() {
  sharedHelper();
}
