// Interfaces and members should be defined.
var test = new JSWebIDLInterface();
JSWebIDLInterface.JS_WEBIDL_CONST;
test.jsWebIDLAttr;
test.jsWebIDLMethod();

// Dictionaries shouldn't be defined, but field should be defined..
typeof JSWebIDLDictionary;
var v = options.jsWebIDLDictionaryProp;

// Enums shouldn't be defined.
typeof JSWebIDLEnum;
typeof js_webidl_enum1;

// Interface mixins shouldn't be defined.
typeof JSWebIDLMixin;

// Interface mixins members should be defined.
JSWebIDLInterface.JS_WEBIDL_MIXIN_CONST;
test.jsWebIDLMixinAttr;
test.jsWebIDLMixinMethod();

// Callbacks shouldn't be defined.
typeof JSWebIDLCallback;

// Namespaces and members should be defined.
JSWebIDLNamespace.JS_WEBIDL_CONST2;
JSWebIDLNamespace.jsWebIDLFunc();

// Overload methods should be defined, but cannot have "Go to".
test.jsWebIDLOverload(1);
test.jsWebIDLOverload(1, 2);

// Partial definitions shouldn't affect the original definition
new JSWebIDLPartialInterface();
JSWebIDLPartialNamespace;

// Members with the same name cannot have "Go to".
var c1 = new JSWebIDLConflicting1();
c1.jsWebIDLConflictAttr;
c1.jsWebIDLConflictMethod();
var c2 = new JSWebIDLConflicting2();
c2.jsWebIDLConflictAttr;
c2.jsWebIDLConflictMethod();
