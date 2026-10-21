"use strict";

// The crash view (the `crash-signature:` query term's; see
// tools/src/crash_stack.rs): a crash signature's stack beside its frames'
// code, innermost frame first, with the lines each frame is believed to have
// been at (its calls of the frame above it) highlighted; and the DWIM rule
// sending crash-stats signatures' URLs there.

function doc() {
  return frame.contentDocument;
}

function frameSection(index) {
  return doc().querySelector(`#frame-${index}`);
}

function crashLines(section) {
  return [...section.querySelectorAll(".crash-line [data-line-number]")]
    .map(lineno => Number(lineno.dataset.lineNumber));
}

add_task(async function test_CrashStackSignature() {
  // (rollup/engine.cpp's Engine::Run calls rollup/storage.cpp's Storage::Put
  // twice; the last frame is a system library's.)
  const signature = "OOM | small | rollup::Storage::Put | rollup::Engine::Run | libc.so.6@0x29d90";
  await TestUtils.loadQuery("tests", `crash-signature:'${signature}'`);

  const annotations = [...doc().querySelectorAll(".crash-annotations code")].map(c => c.textContent);
  is(annotations.join(" | "), "OOM | small", "The signature's annotations aren't frames");
  is(doc().querySelectorAll(".crash-frame").length, 3, "but its 3 frames are");

  is(frameSection(0).querySelector(".crash-code-heading a")?.textContent, "rollup/storage.cpp:5",
     "The innermost frame shows its function's code");
  is(crashLines(frameSection(0)).length, 0, "without a line, as where in it is unknown");

  is(frameSection(1).querySelector(".crash-frame-name")?.textContent, "rollup::Engine::Run",
     "The next frame is its caller");
  is(crashLines(frameSection(1)).join(","), "8,9", "with its calls of it highlighted");
  ok(frameSection(1).querySelector(".crash-frame-where").textContent.includes("lines 8, 9"),
     "and said");

  ok(frameSection(2).querySelector(".crash-frame-unresolved"),
     "A frame that isn't the tree's function is shown as it is");
  ok(frameSection(2).querySelector(".crash-code-missing"), "without code");
});

add_task(async function test_CrashStackInlined() {
  // (rollup/tests/storage_test.cpp's RunStorageTest calls StorageTest::Run,
  // which calls Storage::Put: a stack without StorageTest::Run's frame, as if
  // inlined, highlights RunStorageTest's call of it.)
  const signature = "rollup::Storage::Put | rollup::RunStorageTest";
  await TestUtils.loadQuery("tests", `crash-signature:'${signature}'`);
  is(crashLines(frameSection(1)).join(","), "18",
     "A frame's call of a function calling the frame above is highlighted");
  ok(frameSection(1).querySelector(".crash-frame-where code")?.textContent.includes("StorageTest::Run"),
     "naming the function it calls");
});

add_task(async function test_CrashStackDwim() {
  // (A search for a crash-stats signature's URL, which finds nothing, goes to
  // its crash view's query, in a tree searching with queries.)
  const url = "https://crash-stats.mozilla.org/signature/?product=Firefox&signature=Widget%3A%3AFrob%20%7C%20main";
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(url)}`);
  is(doc().location.pathname, "/trydelta/query/default", "A crash-stats signature's URL goes to");
  is(new URLSearchParams(doc().location.search).get("q"), "crash-signature:'Widget::Frob | main'",
     "the query of its signature's stack");
  ok(doc().querySelector(".crash-signature")?.href.startsWith("https://crash-stats.mozilla.org/signature/"),
     "which links to it on crash-stats");
});
