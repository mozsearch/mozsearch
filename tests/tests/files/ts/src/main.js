function fib(n) {
  if (n <= 1) {
    return 0
  }
  return fib(n - 1) + fib(n - 2)
}

function print_fib(a) {
  console.log(fib(a))
}

var y = 'Hello'
function capture() {
  return y
}
const capture_lambda = () => {
  return y
}

for (var i = 0; i <= 10; i++) {}

for (const x of [1, 2, 3]) {
}

var a = 0
var a = 1
print_fib(a)

function forever() {
  return forever()
}

function use_before_def() {
  print_fib(n)
  var n = 10

  if (forever()) {
    var m = 10
  }
  print_fib(m)
}

function var_function_scope() {
  var k = 0
  if (forever()) {
    var k = 1
  }
  print_fib(k)
}

function array_of_objects() {
  var a = [{ element: 0 }, { element: 1 }]
}

function SomeClass() {}

SomeClass.prototype = {
  someMethod() {},
}

SomeClass.prototype.someMethod2 = () => {}

new SomeClass().someMethod()
new SomeClass().someMethod2()
