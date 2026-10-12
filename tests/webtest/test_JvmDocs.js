"use strict";

// External JVM symbols (ones the SCIP index uses but doesn't define, ex: the
// Java APIs Android provides) link to their documentation (see
// tools/src/jvm_docs.rs).
add_task(async function test_java_docs() {
  await TestUtils.loadPath("/tests/source/src/main/java/sample/JdkUser.java");
  const span = frame.contentDocument.querySelector(
    `span[data-symbols="S_jvm_java/lang/Integer#MAX_VALUE."]`);
  ok(span, "Integer.MAX_VALUE has its symbol");
  TestUtils.click(span);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown");
  const docs = [...menu.querySelectorAll("a")].find(a => a.textContent.includes("Android documentation"));
  is(docs?.getAttribute("href"),
     "https://developer.android.com/reference/java/lang/Integer#MAX_VALUE",
     "It links to the field's documentation in Android's reference");
});
