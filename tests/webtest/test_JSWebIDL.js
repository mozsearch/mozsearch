"use strict";

function findMenuLink(menu, text) {
  for (const row of menu.querySelectorAll(".contextmenu-link")) {
    if (row.textContent.includes(text)) {
      return row;
    }
  }

  return null;
}

add_task(async function test_JSDefinitionInWebIDL() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const tests = [
    {
      line: 2,
      sym: "#JSWebIDLInterface",
      pretty: "JSWebIDLInterface",
      kind: "interface",
      defLine: 1,
    },
    {
      line: 3,
      sym: "#JS_WEBIDL_CONST",
      pretty: "JSWebIDLInterface::JS_WEBIDL_CONST",
      kind: "const",
      defLine: 4,
    },
    {
      line: 4,
      sym: "#jsWebIDLAttr",
      pretty: "JSWebIDLInterface::jsWebIDLAttr",
      kind: "attribute",
      defLine: 5,
    },
    {
      line: 5,
      sym: "#jsWebIDLMethod",
      pretty: "JSWebIDLInterface::jsWebIDLMethod",
      kind: "method",
      defLine: 6,
    },
    {
      line: 8,
      sym: "#JSWebIDLDictionary",
      pretty: "JSWebIDLDictionary",
      noGoto: true,
    },
    {
      line: 9,
      sym: "#jsWebIDLDictionaryProp",
      pretty: "JSWebIDLDictionary.jsWebIDLDictionaryProp",
      kind: "member",
      defLine: 13,
    },
    {
      line: 12,
      sym: "#JSWebIDLEnum",
      pretty: "JSWebIDLEnum",
      noGoto: true,
    },
    {
      line: 13,
      sym: "#js_webidl_enum1",
      pretty: "js_webidl_enum1",
      noGoto: true,
    },
    {
      line: 16,
      sym: "#JSWebIDLMixin",
      pretty: "JSWebIDLMixin",
      noGoto: true,
    },
    {
      line: 19,
      sym: "#JS_WEBIDL_MIXIN_CONST",
      pretty: "JSWebIDLMixin::JS_WEBIDL_MIXIN_CONST",
      kind: "const",
      defLine: 21,
    },
    {
      line: 20,
      sym: "#jsWebIDLMixinAttr",
      pretty: "JSWebIDLMixin::jsWebIDLMixinAttr",
      kind: "attribute",
      defLine: 22,
    },
    {
      line: 21,
      sym: "#jsWebIDLMixinMethod",
      pretty: "JSWebIDLMixin::jsWebIDLMixinMethod",
      kind: "method",
      defLine: 23,
    },
    {
      line: 24,
      sym: "#JSWebIDLCallback",
      pretty: "JSWebIDLCallback",
      noGoto: true,
    },
    {
      line: 27,
      sym: "#JSWebIDLNamespace",
      pretty: "JSWebIDLNamespace",
      kind: "namespace",
      defLine: 28,
    },
    {
      line: 27,
      sym: "JSWebIDLNamespace#JS_WEBIDL_CONST2",
      pretty: "JSWebIDLNamespace::JS_WEBIDL_CONST2",
      kind: "const",
      defLine: 29,
    },
    {
      line: 28,
      sym: "JSWebIDLNamespace#jsWebIDLFunc",
      pretty: "JSWebIDLNamespace::jsWebIDLFunc",
      kind: "method",
      defLine: 30,
    },
    {
      line: 31,
      sym: "#jsWebIDLOverload",
      pretty: "JSWebIDLInterface::jsWebIDLOverload",
      kind: "method",
      noGoto: true,
    },
    {
      line: 32,
      sym: "#jsWebIDLOverload",
      pretty: "JSWebIDLInterface::jsWebIDLOverload",
      kind: "method",
      noGoto: true,
    },
    {
      line: 35,
      sym: "#JSWebIDLPartialInterface",
      pretty: "JSWebIDLPartialInterface",
      kind: "interface",
      noGoto: true
    },
    {
      line: 38,
      sym: "#JSWebIDLPartialNamespace",
      pretty: "JSWebIDLPartialNamespace",
      kind: "namespace",
      noGoto: true
    },
    {
      line: 43,
      sym: "#jsWebIDLConflictAttr",
      pretty: "JSWebIDLConflicting1::jsWebIDLConflictAttr",
      kind: "attribute",
      defLine: 50,
    },
    {
      line: 43,
      sym: "#jsWebIDLConflictAttr",
      pretty: "JSWebIDLConflicting2::jsWebIDLConflictAttr",
      kind: "attribute",
      defLine: 55,
    },
    {
      line: 44,
      sym: "#jsWebIDLConflictMethod",
      pretty: "JSWebIDLConflicting1::jsWebIDLConflictMethod",
      kind: "method",
      defLine: 51,
    },
    {
      line: 44,
      sym: "#jsWebIDLConflictMethod",
      pretty: "JSWebIDLConflicting2::jsWebIDLConflictMethod",
      kind: "method",
      defLine: 56,
    },
    {
      line: 47,
      sym: "#jsWebIDLConflictAttrMany",
      pretty: "jsWebIDLConflictAttrMany",
      noGoto: true,
    },
  ];

  for (const { line, sym, pretty, defLine, kind, noGoto=false } of tests) {
    const selector = `#line-${line} span[data-symbols*="${sym}"]`;
    const elem = frame.contentDocument.querySelector(selector);
    ok(!!elem, `Symbol element exists for ${sym}`);
    TestUtils.click(elem);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

    const label = `Go to IDL definition of ${pretty}`;
    const link = findMenuLink(menu, label);
    if (noGoto) {
      ok(!link, "Menu item with " + label);
    } else {
      ok(!!link, "Menu item with " + label);
      is(link.getAttribute("href"), `/tests/source/webidl/js.webidl#${defLine}`,
         `Menu item should link to the definition at line ${defLine}`);
    }

    if (kind) {
      const label2 = `Search for IDL ${kind} ${pretty}`;
      const link2 = findMenuLink(menu, label2);
      ok(!!link2, "Menu item with " + label2);
    }
  }
});

add_task(async function test_JSDefinitionInWebIDL_overload() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 31;
  const sym = "#jsWebIDLOverload";
  const pretty = "JSWebIDLInterface::jsWebIDLOverload";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const label = `Search for IDL method ${pretty}`;
  const link = findMenuLink(menu, label);
  ok(!!link, "Menu item with " + label);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#8"]`),
     "The first overload should be linked");
  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#9"]`),
     "The second overload should be linked");
});

add_task(async function test_JSDefinitionInWebIDL_partial_interface() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 35;
  const sym = "#JSWebIDLPartialInterface";
  const pretty = "JSWebIDLPartialInterface";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const label = `Search for IDL interface ${pretty}`;
  const link = findMenuLink(menu, label);
  ok(!!link, "Menu item with " + label);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#33"]`),
     "The interface should be linked");
  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#37"]`),
     "The partial interface should be linked");
});

add_task(async function test_JSDefinitionInWebIDL_partial_namespace() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 38;
  const sym = "#JSWebIDLPartialNamespace";
  const pretty = "JSWebIDLPartialNamespace";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const label = `Search for IDL namespace ${pretty}`;
  const link = findMenuLink(menu, label);
  ok(!!link, "Menu item with " + label);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#41"]`),
     "The namespace should be linked");
  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#45"]`),
     "The partial namespace should be linked");
});
