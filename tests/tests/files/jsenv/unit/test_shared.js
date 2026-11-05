// A test running under xpcshell (xpcshell.toml, after head_jsenv.js) and in
// a mochitest page (../test_shared.html, before ../helpers.js), whose
// sharedHelper is a different function in each.
function run_test() {
  sharedHelper();
}
