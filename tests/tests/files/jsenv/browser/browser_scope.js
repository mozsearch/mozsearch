// A browser-chrome test, whose harness's names are its scope's members.
add_task(async function test_scope() {
  ok(true, "the scope's ok");
  browserHelper();
});

// Its DOM is the tree's lib replacement's (../types/), whose members are
// bound to the tree's WebIDL, as Firefox's browser-chrome tests' is Gecko's.
add_task(async function test_dom() {
  new Gadget().poke();
});

