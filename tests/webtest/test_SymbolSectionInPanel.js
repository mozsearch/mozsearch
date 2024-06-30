"use strict";

add_task(async function test_SymbolSectionInPanel() {
  await TestUtils.resetFeatureGate("fancyBar");
  await TestUtils.loadPath("/tests/source/webtest/CopyAsMarkdown.cpp");

  // Hook the clipboard API for 2 reasons:
  //   * in order to check the copied text
  //   * given there's no user activity, the original writeText will throw
  let copiedText = null;
  frame.contentWindow.navigator.clipboard.writeText = async function(text) {
    copiedText = text;
  };

  const symBox = frame.contentDocument.querySelector(".selected-symbol-box");

  is(symBox.textContent,
     "(no symbol clicked)",
     "No symbol is shown when nothing is selected");

  // Select the top level comment.
  TestUtils.selectLine(3);

  is(symBox.textContent,
     "(no symbol clicked)",
     "No symbol is shown when nothing is selected");

  // Select the global variable.
  TestUtils.selectLine(5);

  is(symBox.textContent,
     "globalVariable",
     "Selected global variable name is shown");

  // Select class name.
  TestUtils.selectLine(11);

  is(symBox.textContent,
     "copy_as_markdown::CopyAsMarkdown",
     "Selected class's qualified name is shown");

  // Click method name.
    const methodName = frame.contentDocument.querySelector(`span.syn_def[data-symbols="_ZN16copy_as_markdown14CopyAsMarkdown10SomeMethodEv"]`);
  TestUtils.click(methodName);

  is(symBox.textContent,
     "copy_as_markdown::CopyAsMarkdown::SomeMethod",
     "Selected method's qualified name is shown");

  const copyButton = frame.contentDocument.querySelector(".copy-box .indicator");
  TestUtils.click(copyButton);
  is(copiedText,
     "copy_as_markdown::CopyAsMarkdown::SomeMethod",
     "The selected symbol is copied");
});

add_task(async function test_SymbolSectionInPanel_gate() {
  registerCleanupFunction(async () => {
    await TestUtils.resetFeatureGate("fancyBar");
  });

  const tests = [
    { value: "release", shown: false },
    { value: "beta", shown: false },
    { value: "alpha", shown: true },
  ];
  for (const { value, shown } of tests) {
    await TestUtils.setFeatureGate("fancyBar", value);
    await TestUtils.loadPath("/tests/source/webtest/CopyAsMarkdown.cpp");

    const symBox = frame.contentDocument.querySelector(".selected-symbol-box");
    if (shown) {
      ok(!!symBox, "Symbol section should be shown if enabled");
    } else {
      ok(!symBox, "Symbol section should be shown if disabled");
    }
  }
});
