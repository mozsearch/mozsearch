"use strict";

add_task(async function test_Titler() {
  registerCleanupFunction(async () => {
    await TestUtils.setTitleBehavior("lineSelection", true);
    await TestUtils.setTitleBehavior("stickySymbol", true);
  });
  const tests = [
    { lineSelection: false, stickySymbol: false },
    { lineSelection: false, stickySymbol: true },
    { lineSelection: true, stickySymbol: false },
    { lineSelection: true, stickySymbol: true },
  ];

  for (const { lineSelection, stickySymbol } of tests) {
    await TestUtils.setTitleBehavior("lineSelection", lineSelection);
    await TestUtils.setTitleBehavior("stickySymbol", stickySymbol);
    await TestUtils.loadPath("/tests/source/webtest/Titler.cpp");

    is(frame.contentDocument.title,
       "Titler.cpp - mozsearch",
       "Filename is shown in the title");

    TestUtils.selectLine(1);
    if (lineSelection) {
      is(frame.contentDocument.title,
         "globalVariable (Titler.cpp - mozsearch)",
         "Symbol in the selected line is shown in the title if enabled");
    } else {
      is(frame.contentDocument.title,
         "Titler.cpp - mozsearch",
         "Symbol in the selected line is not shown in the title if disabled");
    }

    TestUtils.selectLine(1, { shiftKey: true, bubbles: true });
    is(frame.contentDocument.title,
       "Titler.cpp - mozsearch",
       "Symbol disappears after unselecting the line.");

    frame.contentDocument.documentElement.scrollTop =
      frame.contentDocument.documentElement.scrollHeight;

    if (stickySymbol) {
      await waitForCondition(
        () => frame.contentDocument.title == "pagetitler (Titler.cpp - mozsearch)",
        "Sticky symbol is shown in the title");
    } else {
      await TestUtils.sleep(100);
      is(frame.contentDocument.title,
         "Titler.cpp - mozsearch",
         "Sticky symbol is not shown in the title if disabled");
    }

    TestUtils.selectLine(190);
    if (lineSelection) {
      is(frame.contentDocument.title,
         "p:PageTitler (Titler.cpp - mozsearch)",
         "Symbol in the selected line is shown with shortened namespace in the title if enabled");
    } else if (stickySymbol) {
      await waitForCondition(
        () => frame.contentDocument.title == "pagetitler (Titler.cpp - mozsearch)",
        "Sticky symbol is shown in the title but symbol in the selected line is not shown if disabled");
    } else {
      is(frame.contentDocument.title,
         "Titler.cpp - mozsearch",
         "Symbol in the selected line is not shown in the title if disabled");
    }
  }
});
