"use strict";

function sanitizeURL(text) {
  return text.replace(/https?:\/\/[^\/]+\//, "BASE_URL/").replace(/\/rev\/[^\/]+\//, "/rev/REV/");
}

add_task(async function test_CopyAsMarkdown() {
  await TestUtils.loadPath("/tests/source/webtest/CopyAsMarkdown.cpp");

  const clipboard = TestUtils.spyClipboard();

  const filenameButton = frame.contentDocument.querySelector("#panel-copy-filename-link");
  const symbolButton = frame.contentDocument.querySelector("#panel-copy-symbol-link");
  const codeButton = frame.contentDocument.querySelector("#panel-copy-code-block");
  const symbolBlockButton = frame.contentDocument.querySelector("#panel-copy-symbol-block");

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should always be enabled (for the file, without a symbol)");
  ok(codeButton.disabled, "Code Block should be disabled if nothing is selected");
  ok(symbolBlockButton.disabled, "Symbol Link and Code Block should be disabled if nothing is selected");

  TestUtils.click(filenameButton);
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp)",
     "Filename is copied");

  clipboard.value = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "F" });
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp)",
     "Filename is copied");

  clipboard.value = "";
  TestUtils.click(symbolButton);
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp)",
     "Without a symbol, Symbol Link links the file");
  clipboard.value = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp)",
     "Without a symbol, Symbol Link links the file, by its accel too");

  clipboard.value = "*unmodified*";
  TestUtils.click(codeButton);
  is(clipboard.value, "*unmodified*", "Copy does not happen for disabled Code Block button");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(clipboard.value, "*unmodified*", "Copy does not happen for disabled Code Block accel");

  // Select the top level comment.
  TestUtils.selectLine(3);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled (for the file and lines) if no symbol is selected");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");
  ok(!symbolBlockButton.disabled, "Symbol Link and Code Block should be enabled when a line is selected");

  TestUtils.click(codeButton);
  is(sanitizeURL(clipboard.value),
     "BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#3\n" +
     "```cpp\n" +
     "// Comment at the top level.\n" +
     "```",
     "Code block with single line is copied");

  clipboard.value = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(sanitizeURL(clipboard.value),
     "BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#3\n" +
     "```cpp\n" +
     "// Comment at the top level.\n" +
     "```",
     "Code block with single line is copied");

  TestUtils.click(symbolButton);
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp#3](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#3)",
     "Without a symbol, Symbol Link links the file and lines");

  clipboard.value = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "B" });
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp#3](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#3)\n" +
     "```cpp\n" +
     "// Comment at the top level.\n" +
     "```",
     "Symbol Link and Code Block leads the code block with the link");

  // Select the global variable.
  TestUtils.selectLine(5);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when a symbol exists in the selected line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(clipboard.value),
     "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
     "Global variable symbol is copied");

  TestUtils.click(symbolBlockButton);
  is(sanitizeURL(clipboard.value),
     "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)\n" +
     "```cpp\n" +
     "bool globalVariable = true;\n" +
     "```",
     "Symbol Link and Code Block leads the code block with the symbol's link");

  // "Copy as..." previews each format, to copy.
  TestUtils.click(frame.contentDocument.querySelector("#panel-copy-as"));
  const popup = frame.contentDocument.querySelector(".copy-as-popup");
  ok(popup, "Copy as... opens its popup");
  const rows = [...popup.querySelectorAll("li")];
  is(JSON.stringify(rows.map(row => row.querySelector("strong").textContent)),
     JSON.stringify(["Symbol link", "Symbol link and code block", "Code block", "File link"]),
     "The popup has the formats");
  is(sanitizeURL(rows[0].querySelector(".copy-as-preview").textContent),
     "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
     "with previews");
  clipboard.value = "";
  TestUtils.click(rows[3].querySelector(".copy-as-copy"));
  is(sanitizeURL(clipboard.value),
     "[CopyAsMarkdown.cpp](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
     "whose Copy buttons copy them");
  TestUtils.click(popup.querySelector(".lens-popup-close"));

  clipboard.value = "";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(sanitizeURL(clipboard.value),
     "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
     "Global variable symbol is copied");

  // Select the namespace.

  TestUtils.selectLine(7);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(clipboard.value),
     "[copy_as_markdown](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#7)",
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
  is(sanitizeURL(clipboard.value),
     "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#14)",
     "Method symbol is copied");

  // Select the local variable
  TestUtils.selectLine(17);

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected line is inside a nesting line");
  ok(!codeButton.disabled, "Code Block should be enabled when a line is selected");

  TestUtils.click(symbolButton);
  is(sanitizeURL(clipboard.value),
     "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#17)",
     "Method symbol is copied instead of local variable symbol");

  // Shift-select from the start of the class to the currently selected local variable.
  TestUtils.selectLine(11, { bubbles: true, shiftKey: true });

  ok(!filenameButton.disabled, "Filename Link should always be enabled");
  ok(!symbolButton.disabled, "Symbol Link should be enabled when the selected lines have symbol");
  ok(!codeButton.disabled, "Code Block should be enabled when lines are selected");

  TestUtils.click(codeButton);
  is(sanitizeURL(clipboard.value),
     "BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#11-17\n" +
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
  is(sanitizeURL(clipboard.value),
     "BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#14-15,17\n" +
     "```cpp\n" +
     "void SomeMethod() {\n" +
     "  // Comment inside method.\n" +
     "...\n" +
     "  bool LocalVariable = true;\n" +
     "```",
     "Code block with multiple lines are copied with dedent");

  // Disable accel (the settings page's setting).

  frame.contentWindow.localStorage.setItem("accel-enable", "0");
  registerCleanupFunction(() => {
    frame.contentWindow.localStorage.removeItem("accel-enable");
  });

  clipboard.value = "*unmodified*";
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "F" });
  is(clipboard.value, "*unmodified*", "Copy does not happen if accel is disabled");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "S" });
  is(clipboard.value, "*unmodified*", "Copy does not happen if accel is disabled");
  TestUtils.keypress(frame.contentDocument.documentElement, { bubbles: true, key: "C" });
  is(clipboard.value, "*unmodified*", "Copy does not happen if accel is disabled");
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

    const symbolButton = frame.contentDocument.querySelector("#panel-copy-symbol-link");

    const clipboard = TestUtils.spyClipboard();

    TestUtils.selectLine(5);

    TestUtils.click(symbolButton);
    is(sanitizeURL(clipboard.value),
       "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
       "Global variable symbol is copied");

    const methodName = frame.contentDocument.querySelector(`span.syn_def[data-symbols="_ZN16copy_as_markdown14CopyAsMarkdown10SomeMethodEv"]`);
    TestUtils.click(methodName);

    clipboard.value = "*unmodified*";
    TestUtils.click(symbolButton);
    if (enabled) {
      is(sanitizeURL(clipboard.value),
         "[copy_as_markdown::CopyAsMarkdown::SomeMethod](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5,14)",
         "Method symbol is copied if clicked-symbol is enabled, with global variable line number in URL");
    } else {
      is(sanitizeURL(clipboard.value),
         "[globalVariable](BASE_URL/tests/rev/REV/webtest/CopyAsMarkdown.cpp#5)",
         "Global variable symbol is copied if clicked-symbol is disabled");
    }
  }
});
