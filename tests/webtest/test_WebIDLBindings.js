"use strict";

function findMenuItem(menu, text) {
  for (const row of menu.querySelectorAll(".contextmenu-row")) {
    if (row.textContent.includes(text)) {
      return row;
    }
  }

  return null;
}

add_task(async function test_WebIDLBindings() {
  await TestUtils.loadPath("/tests/source/webidl/BindingTest.webidl");

  const tests = [
    {
      sym: "WEBIDL_BindingTest",
      items: [
        "mozilla::dom::BindingTest_Binding",
      ],
    },
    {
      sym: "WEBIDL_BindingTest_constructor",
      items: [
        "mozilla::dom::BindingTest_Binding::_constructor",
      ],
    },
    {
      sym: "WEBIDL_BindingTest_CONST_1",
      items: [
        "mozilla::dom::BindingTest_Binding::CONST_1",
      ],
    },
    {
      sym: "WEBIDL_BindingTest_attr1",
      items: [
        "mozilla::dom::BindingTest_Binding::get_attr1",
        "mozilla::dom::BindingTest_Binding::set_attr1",
        "mozilla::dom::BindingTest::GetAttr1",
        "mozilla::dom::BindingTest::SetAttr1",
      ],
    },
    {
      sym: "WEBIDL_BindingTest_method1",
      items: [
        "mozilla::dom::BindingTest_Binding::method1",
        "mozilla::dom::BindingTest::Method1",
      ],
    },
    {
      sym: "WEBIDL_BindingTestDict",
      items: [
        "mozilla::dom::BindingTestDict",
      ],
    },
    {
      sym: "WEBIDL_BindingTestDict_prop1",
      items: [
        "mozilla::dom::BindingTestDict::mProp1",
      ],
    },
    {
      sym: "WEBIDL_BindingTestEnum",
      items: [
        "mozilla::dom::BindingTestEnum",
      ],
    },
  ];
  for (const { sym, items } of tests) {
    const elem = frame.contentDocument.querySelector(`span.syn_def[data-symbols="${sym}"]`);
    ok(!!elem, `Symbol element exists for ${sym}`);

    TestUtils.click(elem);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

    for (const item of items) {
      const row = findMenuItem(menu, item);
      ok(!!row, `Menu item for ${item} exists`);
    }
  }
});

add_task(async function test_WebIDLBindingsWithMixin() {
  await TestUtils.loadPath("/tests/source/webidl/BindingTestMixin.webidl");

  const tests = [
    {
      sym: "WEBIDL_BindingTestMixin_MIXIN_CONST",
      items: [
        "mozilla::dom::BindingTestMixed1_Binding::MIXIN_CONST",
        "mozilla::dom::BindingTestMixed2_Binding::MIXIN_CONST",
      ],
    },
    {
      sym: "WEBIDL_BindingTestMixin_mixinAttr",
      items: [
        "mozilla::dom::BindingTestMixed1_Binding::get_mixinAttr",
        "mozilla::dom::BindingTestMixed1_Binding::set_mixinAttr",
        "mozilla::dom::BindingTestMixed1::GetMixinAttr",
        "mozilla::dom::BindingTestMixed1::SetMixinAttr",
        "mozilla::dom::BindingTestMixed2_Binding::get_mixinAttr",
        "mozilla::dom::BindingTestMixed2_Binding::set_mixinAttr",
        "mozilla::dom::BindingTestMixed2::GetMixinAttr",
        "mozilla::dom::BindingTestMixed2::SetMixinAttr",
      ],
    },
    {
      sym: "WEBIDL_BindingTestMixin_mixinMethod",
      items: [
        "mozilla::dom::BindingTestMixed1_Binding::mixinMethod",
        "mozilla::dom::BindingTestMixed1::MixinMethod",
        "mozilla::dom::BindingTestMixed2_Binding::mixinMethod",
        "mozilla::dom::BindingTestMixed2::MixinMethod",
      ],
    },
  ];
  for (const { sym, items } of tests) {
    const elem = frame.contentDocument.querySelector(`span.syn_def[data-symbols="${sym}"]`);
    ok(!!elem, `Symbol element exists for ${sym}`);

    TestUtils.click(elem);

    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown when clicking ${sym} symbol`);

    for (const item of items) {
      const row = findMenuItem(menu, item);
      ok(!!row, `Menu item for ${item} exists`);
    }
  }
});
