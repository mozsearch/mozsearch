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
      defLine: 1,
    },
    {
      line: 3,
      sym: "#JS_WEBIDL_CONST",
      pretty: "JS_WEBIDL_CONST",
      defLine: 4,
    },
    {
      line: 3,
      sym: "#JS_WEBIDL_CONST",
      pretty: "JSWebIDLInterface.JS_WEBIDL_CONST",
      defLine: 4,
    },
    {
      line: 4,
      sym: "#jsWebIDLAttr",
      pretty: "jsWebIDLAttr",
      defLine: 5,
    },
    {
      line: 5,
      sym: "#jsWebIDLMethod",
      pretty: "jsWebIDLMethod",
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
      pretty: "jsWebIDLDictionaryProp",
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
      pretty: "JS_WEBIDL_MIXIN_CONST",
      defLine: 21,
    },
    {
      line: 20,
      sym: "#jsWebIDLMixinAttr",
      pretty: "jsWebIDLMixinAttr",
      defLine: 22,
    },
    {
      line: 21,
      sym: "#jsWebIDLMixinMethod",
      pretty: "jsWebIDLMixinMethod",
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
      defLine: 28,
    },
    {
      line: 27,
      sym: "#JS_WEBIDL_CONST2",
      pretty: "JS_WEBIDL_CONST2",
      defLine: 29,
    },
    {
      line: 27,
      sym: "JSWebIDLNamespace#JS_WEBIDL_CONST2",
      pretty: "JSWebIDLNamespace.JS_WEBIDL_CONST2",
      defLine: 29,
    },
    {
      line: 28,
      sym: "#jsWebIDLFunc",
      pretty: "jsWebIDLFunc",
      defLine: 30,
    },
    {
      line: 28,
      sym: "JSWebIDLNamespace#jsWebIDLFunc",
      pretty: "JSWebIDLNamespace.jsWebIDLFunc",
      defLine: 30,
    },
    {
      line: 31,
      sym: "#jsWebIDLOverload",
      pretty: "jsWebIDLOverload",
      noGoto: true,
    },
    {
      line: 32,
      sym: "#jsWebIDLOverload",
      pretty: "jsWebIDLOverload",
      noGoto: true,
    },
    {
      line: 35,
      sym: "#JSWebIDLPartialInterface",
      pretty: "JSWebIDLPartialInterface",
      defLine: 33,
    },
    {
      line: 36,
      sym: "#JSWebIDLPartialNamespace",
      pretty: "JSWebIDLPartialNamespace",
      defLine: 39,
    },
    {
      line: 40,
      sym: "#jsWebIDLConflictAttr",
      pretty: "jsWebIDLConflictAttr",
      noGoto: true,
    },
    {
      line: 41,
      sym: "#jsWebIDLConflictMethod",
      pretty: "jsWebIDLConflictMethod",
      noGoto: true,
    },
    {
      line: 43,
      sym: "#jsWebIDLConflictAttr",
      pretty: "jsWebIDLConflictAttr",
      noGoto: true,
    },
    {
      line: 44,
      sym: "#jsWebIDLConflictMethod",
      pretty: "jsWebIDLConflictMethod",
      noGoto: true,
    },
  ];

  for (const { line, sym, pretty, defLine, noGoto=false } of tests) {
    const selector = `#line-${line} span[data-symbols*="${sym}"]`;
    const elem = frame.contentDocument.querySelector(selector);
    ok(!!elem, `Symbol element exists for ${sym}`);
    TestUtils.click(elem);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

    const label = `Go to definition of ${pretty}`;
    const link = findMenuLink(menu, label);
    if (noGoto) {
      ok(!link, "Menu item with " + label);
    } else {
      ok(!!link, "Menu item with " + label);
      is(link.getAttribute("href"), `/tests/source/webidl/js.webidl#${defLine}`,
         "Menu item should link to the definition at line ${defLine}");
    }

    const label2 = `Search for ${pretty}`;
    const link2 = findMenuLink(menu, label2);
    ok(!!link2, "Menu item with " + label2);
  }
});

add_task(async function test_JSDefinitionInWebIDL_overload() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 31;
  const sym = "#jsWebIDLOverload";
  const pretty = "jsWebIDLOverload";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const label = `Search for ${pretty}`;
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

add_task(async function test_JSDefinitionInWebIDL_conflict() {
  await TestUtils.loadPath("/tests/source/webidl/consumer.js");

  const line = 40;
  const sym = "#jsWebIDLConflictAttr";
  const pretty = "jsWebIDLConflictAttr";

  const selector = `#line-${line} span[data-symbols*="${sym}"]`;
  const elem = frame.contentDocument.querySelector(selector);
  ok(!!elem, `Symbol element exists for ${sym}`);
  TestUtils.click(elem);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

  const label = `Search for ${pretty}`;
  const link = findMenuLink(menu, label);
  ok(!!link, "Menu item with " + label);

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#46"]`),
     "The first definition should be linked");
  ok(!!frame.contentDocument.querySelector(`[href="/tests/source/webidl/js.webidl#51"]`),
     "The second definition should be linked");
});
