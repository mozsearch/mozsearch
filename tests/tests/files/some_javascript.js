// random bits of javascript

class Foo {
  static bar(baz) {
    return baz;
  }
}

function Laser() {
}
Laser.prototype = {
  propertyNamedFunction: function(arg1) {

  },

  coolFunctionSyntax(arg1) {

  },

  get getterSyntax() {

  }
};

Laser.prototype.separateAssignedFunc = function(arg1) {
  return 5;
};

Laser.prototype.randoObj = {
  nestedObj: {},

  baz: function() {
    
  }
};

Laser.prototype.randoObj.nestedObj.foo = function(arg1) {

};

Laser.prototype.randoObj.addedOn.bar = function(arg1) {

};

var multiline_backtick = `here \
  is a thing`;

function destructure_with_spreadexpression(foo) {
  const {
    contextmenu,
    ...options
  } = foo;
}
