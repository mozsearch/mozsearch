"use strict";

add_task(async function test_LinNumberInUrl() {
  await TestUtils.loadPath("/tests/source/webtest/Webtest.cpp");

  is(frame.contentDocument.location.hash,
     "",
     "Hash is empty if no line is selected");

  TestUtils.selectLine(1);
  is(frame.contentDocument.location.hash,
     "#1",
     "Hash contains the selected line");

  TestUtils.selectLine(1, { shiftKey: true, bubbles: true });
  is(frame.contentDocument.location.hash,
     "",
     "Hash is empty if no line is selected");

  TestUtils.selectLine(3);
  TestUtils.selectLine(7, { shiftKey: true, bubbles: true });
  is(frame.contentDocument.location.hash,
     "#3-7",
     "Shift click selects line range");

  TestUtils.selectLine(9, { metaKey: true, bubbles: true });
  is(frame.contentDocument.location.hash,
     "#3-7,9",
     "Meta click adds line");

  TestUtils.selectLine(11, { shiftKey: true, bubbles: true });
  is(frame.contentDocument.location.hash,
     "#3-7,9-11",
     "Shift click adds line range from the last clicked line");

  TestUtils.selectLine(5, { metaKey: true, bubbles: true });
  is(frame.contentDocument.location.hash,
     "#3-4,6-7,9-11",
     "Meta click deselects line");

  TestUtils.selectLine(12);
  is(frame.contentDocument.location.hash,
     "#12",
     "Normal click deselects all lines and select the clicked line");
});
