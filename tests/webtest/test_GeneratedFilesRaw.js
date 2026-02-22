"use strict";

add_task(async function test_GeneratedFilesRaw() {
  await TestUtils.loadPath("/tests/source/__GENERATED__/generated.cpp");

  const raw = frame.contentDocument.querySelector("#panel-raw");
  ok(!!raw, `Raw link exists`);

  TestUtils.click(raw);

  await waitForCondition(() =>
    frame.contentDocument.location.href.includes("tests/raw/__GENERATED__/generated.cpp"),
    "raw file is opened"
  );

  // In order to verify the raw response, use fetch API.
  const response = await fetch(frame.contentDocument.location.href);
  const text = await response.text();
  is(text,
     `#include "generated.h"\n#include "nsISupports.h"\n\nint generated_func() {\n  return 0;\n}\n`,
     "Raw generated file is returned");
});
