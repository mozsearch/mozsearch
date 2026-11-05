// The xpcshell tests' helper (see xpcshell.toml's head).
function sharedHelper() {
  return "xpcshell";
}

// The head's call of its test's run_test, which it looks up by its name in
// their global (a hook, whose definitions are its tests').
function runTests() {
  if (typeof run_test == "function") {
    run_test();
  }
}
