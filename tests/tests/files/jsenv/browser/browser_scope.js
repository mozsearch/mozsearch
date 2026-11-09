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

// A variable which one task assigns is of the assigned value's type in the
// others, and a function's parameter of its calls' arguments' (with
// scip-typescript's TypeScript; see its scripts/patch-typescript.cjs).
let gadget;
add_task(async function setup_gadget() {
  gadget = new Gadget();
});
add_task(async function test_assigned() {
  gadget.poke();
});
function pokeIt(thing) {
  thing.poke();
}
add_task(async function test_parameter() {
  pokeIt(new Gadget());
});
