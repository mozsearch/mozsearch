const lazy = {};

// LazyImport* shouldn't be treated as definition.
// The consumers should refer the canonical definition instead.
//
// LazyPref*/LazyService*/LazyFunc* should be treated as definition,
// given the property name is specific to the lazy object.
//
// LazyUnknown* shouldn't be affected by the rule here, and should be
// treated as definition.

XPCOMUtils.defineLazy(lazy, {
  LazyImport1: "resource://module.sys.mjs",
  LazyPref1: { pref: "some.pref", default: false },
  LazyService1: { service: "@mozilla.org/something;1", iid: Ci.nsISomething },
  LazyFunc1: () => {},
});

const lazy2 = XPCOMUtils.declareLazy({
  LazyImport2: "resource://module.sys.mjs",
  LazyPref2: { pref: "some.pref", default: false },
  LazyService2: { service: "@mozilla.org/something;1", iid: Ci.nsISomething },
  LazyFunc2: () => {},
});

XPCOMUtils.defineLazyServiceGetters(lazy, {
  LazyService3: ["@mozilla.org/something;1", Ci.nsISomething],
});

ChromeUtils.defineESModuleGetters(lazy, {
  LazyImport3: "resource://module.sys.mjs",
});

UnknownFunction(lazy, {
  LazyUnknown1: "resource://module.sys.mjs",
});

lazy.LazyImport1;
lazy.LazyPref1;
lazy.LazyService1;
lazy.LazyFunc1;

lazy2.LazyImport2;
lazy2.LazyPref2;
lazy2.LazyService2;
lazy2.LazyFunc2;

lazy.LazyService3;

lazy.LazyImport3;

lazy.LazyUnknown1;
