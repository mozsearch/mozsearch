// random bits of javascript

const str_computed_field_num = 'computed_field_num';

class ClassWithProperties {

  pub_field_undefined;

  pub_field_num = 1;

  pub_field_dict = {
    field_sub_prop: 2,
    field_sub_dict: {
      field_sub_sub_prop: 3
    }
  };

  pub_field_func = function(b) {
    return b * 2;
  };

  pub_field_arrow_func = b => {
    return b * 2;
  };

  // Computed property name syntax.
  [str_computed_field_num] = 3;

  // PRIVATE FIELDS AREN'T SUPPORTED YET.
  // These next 2 simply won't emit anything in the AST right now.
  /*
  #priv_field_undefined;

  #priv_field_num = 10;
  */

  // This generates a syntax error even if I wrap the object initializer in
  // parens.  The spec is very dry and I'm having trouble figuring out if this
  // is actually legal or not.
  /*
  #priv_field_dict = {
    sub_prop: 12
  };
  */

  // This also doesn't work.
  /*
  #priv_field_func = b => {
    return b * 2;
  };
  */

  // Disabled by bug 1559269 for now.
  /*
  consumes_priv_field_num() {
    return this.#priv_field_num;
  }
  */
}

class ClassWithStaticMethods {
  static theStaticMethod(baz) {
    return baz;
  }
}

let obj_dict = {
  obj_prop: 2,
  obj_sub_dict: {
    obj_sub_prop: 3,
    obj_sub_sub_dict: {
      obj_sub_sub_prop: 4
    }
  }
};

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
