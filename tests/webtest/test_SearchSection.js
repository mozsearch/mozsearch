"use strict";

add_task(async function test_CollapseSearchSection() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "SimpleSearch");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("class SimpleSearch"),
    "1 class matches");

  const expando = frame.contentDocument.querySelector(".expando");
  ok(!!expando, "Expando button exists");
  ok(expando.classList.contains("open"), "Expando button is opened");

  const resultHead = frame.contentDocument.querySelector(".result-head");
  ok(!!resultHead, "Result head exists");
  ok(TestUtils.isShown(resultHead), "Result head is expanded");

  TestUtils.click(expando);

  ok(!expando.classList.contains("open"), "Expando button is not open");
  ok(!TestUtils.isShown(resultHead), "Result head is collapsed");
});
