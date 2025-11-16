interface JSWebIDLInterface {
  constructor();

  const unsigned long JS_WEBIDL_CONST = 10;
  attribute any jsWebIDLAttr;
  any jsWebIDLMethod();

  any jsWebIDLOverload(any a1);
  any jsWebIDLOverload(any a1, any a2);
};

dictionary JSWebIDLDictionary {
  long jsWebIDLDictionaryProp;
};

enum JSWebIDLEnum {
  "js_webidl_enum1",
};

interface mixin JSWebIDLMixin {
  const unsigned long JS_WEBIDL_MIXIN_CONST = 20;
  attribute any jsWebIDLMixinAttr;
  any jsWebIDLMixinMethod();
};

JSWebIDLInterface includes JSWebIDLMixin;

namespace JSWebIDLNamespace {
  const unsigned long JS_WEBIDL_CONST2 = 10;
  any jsWebIDLFunc();
};

interface JSWebIDLPartialInterface {
};

partial interface JSWebIDLPartialInterface {
};

namespace JSWebIDLPartialNamespace {
};

partial namespace JSWebIDLPartialNamespace {
};

interface  JSWebIDLConflicting1 {
  attribute any jsWebIDLConflictAttr;
  any jsWebIDLConflictMethod();
};

interface  JSWebIDLConflicting2 {
  attribute any jsWebIDLConflictAttr;
  any jsWebIDLConflictMethod();
};
