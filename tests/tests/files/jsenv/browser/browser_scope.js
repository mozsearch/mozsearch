// A browser-chrome test, whose harness's names are its scope's members.
add_task(async function test_scope() {
  ok(true, "the scope's ok");
  browserHelper();
});
