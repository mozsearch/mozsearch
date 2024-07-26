var await = 0;
var yield = 0;
var let = 0;
var static = 0;
var implements = 0;
var interface = 0;
var package = 0;
var private = 0;
var protected = 0;
var public = 0;
var as = 0;
var async = 0;
var from = 0;
var get = 0;
var meta = 0;
var of = 0;
var set = 0;
var target = 0;

async function* f() {
  await 10;
  yield 10;
  let x = 10;
}

for (var x of []) {
}

class C {
  constructor() {
    new.target;
  }

  static foo() {}
  get prop() {}
  set prop(v) {}
}
