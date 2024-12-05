"use strict";

function sanitizeURL(text) {
  return text.replace(/https?:\/\/[^\/]+\//, "BASE_URL/");
}

add_task(async function test_CopyAsMarkdown() {
  await TestUtils.loadPath("/tests/source/webtest/CopyAsMarkdown.cpp");

  // Hook the clipboard API for 2 reasons:
  //   * in order to check the copied text
  //   * given there's no user activity, the original writeText will throw
  let copiedText = null;
  frame.contentWindow.navigator.clipboard.writeText = async function(text) {
    copiedText = text;
  };

  const filenameButton = frame.contentDocument.querySelector(`button[title="Filename Link"]`);
  const symbolButton = frame.contentDocument.querySelector(`button[title="Symbol Link"]`);
  const codeButton = frame.contentDocument.querySelector(`button[title="Code Block"]`);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(symbolButton.disabled, "Symbol Link should be disabled if nothing is selected");
  ok(codeButton.disabled, "Code Block should be disabled if nothing is selected");

  TestUtils.click(filenameButton);
  is(sanitizeURL(copiedText),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp)",
     "Filename is copied");

  copiedText = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "F" });
  is(sanitizeURL(copiedText),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp)",
     "Filename is copied");

  copiedText = "*unmodified*";
  TestUtils.click(symbolButton);
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Symbol Link button");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Symbol Link accel");

  TestUtils.click(codeButton);
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Code Block button");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Code Block accel");

  // Select the top level comment.
  TestUtils.selectLine(3);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(symbolButton.disabled, "Symbol Link should be disabled if no symbol is selected");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(codeButton);
  is(copiedText.replace(/https?:\/\/[^\/]+\//, "BASE_URL/"),
     "BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#3\n" +
     "```cpp\n" +
     "// Comment at the top level.\n" +
     "```",
     "Code block with single line is copied");

  copiedText = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(copiedText.replace(/https?:\/\/[^\/]+\//, "BASE_URL/"),
     "BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#3\n" +
     "```cpp\n" +
     "// Comment at the top level.\n" +
     "```",
     "Code block with single line is copied");

  copiedText = "*unmodified*";
  TestUtils.click(symbolButton);
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Symbol Link button");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(copiedText, "*unmodified*", "Copy does not happen for disabled Symbol Link accel");

  // Select the global variable.
  TestUtils.selectLine(5);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when a symbol exists in the selected line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(copiedText),
     "[globalVariable](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#5)",
     "Global variable symbol is copied");

  copiedText = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(sanitizeURL(copiedText),
     "[globalVariable](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#5)",
     "Global variable symbol is copied");

  // Select the namespace.

  TestUtils.selectLine(7);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(copiedText),
     "[copy_as_markdown](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#7)",
     "Namespace symbol is copied");

  // Select the comment inside namespace.
  TestUtils.selectLine(9);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  // Select the method
  TestUtils.selectLine(14);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(copiedText),
     "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#14)",
     "Method symbol is copied");

  // Select the local variable
  TestUtils.selectLine(17);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(copiedText),
     "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#17)",
     "Method symbol is copied instead of local variable symbol");

  // Shift-select from the start of the class to the currently selected local variable.
  TestUtils.selectLine(11, { bubbles: true, shiftKey: true });

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected lines have symbol");
  ok(!codeButton.disabled, "Code Block should be enabled when lines are selected");

  TestUtils.click(codeButton);
  is(copiedText.replace(/https?:\/\/[^\/]+\//, "BASE_URL/"),
     "BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#11-17\n" +
     "```cpp\n" +
     "class CopyAsMarkdown {\n" +
     "  // Comment inside class.\n" +
     "\n" +
     "  void SomeMethod() {\n" +
     "    // Comment inside method.\n" +
     "\n" +
     "    bool LocalVariable = true;\n" +
     "```",
     "Code block with multiple lines are copied");

  // Select lines inside a block
  TestUtils.selectLine(14);
  TestUtils.selectLine(15, { bubbles: true, metaKey: true });
  TestUtils.selectLine(17, { bubbles: true, metaKey: true });

  TestUtils.click(codeButton);
  is(copiedText.replace(/https?:\/\/[^\/]+\//, "BASE_URL/"),
     "BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#14-15,17\n" +
     "```cpp\n" +
     "void SomeMethod() {\n" +
     "  // Comment inside method.\n" +
     "...\n" +
     "  bool LocalVariable = true;\n" +
     "```",
     "Code block with multiple lines are copied with dedent");

  // Disable accel

  const accelCheckbox = frame.contentDocument.querySelector("#panel-accel-enable");
  TestUtils.clickCheckbox(accelCheckbox);
  registerCleanupFunction(() => {
    TestUtils.clickCheckbox(accelCheckbox);
  });

  copiedText = "*unmodified*";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "F" });
  is(copiedText, "*unmodified*", "Copy does not happen if accel is disabled");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(copiedText, "*unmodified*", "Copy does not happen if accel is disabled");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(copiedText, "*unmodified*", "Copy does not happen if accel is disabled");
});

add_task(async function test_CopyAsMarkdown_clicked() {
  registerCleanupFunction(async () => {
    await TestUtils.resetFeatureGate("fancyBar");
  });

  const tests = [
    { value: "release", enabled: false },
    { value: "beta", enabled: false },
    { value: "alpha", enabled: true },
  ];
  for (const { value, enabled } of tests) {
    await TestUtils.setFeatureGate("fancyBar", value);
    await TestUtils.loadPath("/tests/source/webtest/CopyAsMarkdown.cpp");

    const symbolButton = frame.contentDocument.querySelector(`button[title="Symbol Link"]`);

    let copiedText = null;
    frame.contentWindow.navigator.clipboard.writeText = async function(text) {
      copiedText = text;
    };

    TestUtils.selectLine(5);

    TestUtils.click(symbolButton);
    is(sanitizeURL(copiedText),
       "[globalVariable](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#5)",
       "Global variable symbol is copied");

    const methodName = frame.contentDocument.querySelector(`span.syn_def[data-symbols="_ZN16copy_as_markdown14CopyAsMarkdown10SomeMethodEv"]`);
    TestUtils.click(methodName);

    copiedText = "*unmodified*";
    TestUtils.click(symbolButton);
    if (enabled) {
      is(sanitizeURL(copiedText),
         "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#5,14)",
         "Method symbol is copied if clicked-symbol is enabled, with global variable line number in URL");
    } else {
      is(sanitizeURL(copiedText),
         "[globalVariable](BASE_URL/tests/source/webtest/CopyAsMarkdown.cpp#5)",
         "Global variable symbol is copied if clicked-symbol is disabled");
    }
  }
});
