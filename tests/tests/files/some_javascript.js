// random bits of javascript

class Foo {
  static bar(baz) {
    return baz;
  }
}

var multiline_backtick = `here \
  is a thing`;

function destructure_with_spreadexpression(foo) {
  const {
    contextmenu,
    ...options
  } = foo;
}
