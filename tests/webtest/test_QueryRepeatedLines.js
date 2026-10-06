"use strict";

// /query/'s textual occurrences on lines that other results already show (for
// their other matches, ex: "Dispatcher" in `Run(Dispatcher* aDispatcher)`,
// whose type is a use) are hidden unless the checkbox in the textual
// occurrences' heading includes them, which is remembered.

const SHOW_REPEATED_KEY = "query-show-repeated";

function textualOccurrences() {
  return [...frame.contentDocument.querySelectorAll(".query-result details.facet-group")]
    .find(group => group.querySelector(":scope > summary > h2 .query-toggle")?.textContent == "Textual Occurrences");
}

add_task(async function test_QueryRepeatedLines() {
  await TestUtils.loadQuery("tests", "Dispatcher");
  frame.contentWindow.localStorage.removeItem(SHOW_REPEATED_KEY);
  registerCleanupFunction(() => frame.contentWindow.localStorage.removeItem(SHOW_REPEATED_KEY));
  await TestUtils.loadQuery("tests", "Dispatcher");

  let text = textualOccurrences();
  ok(text, "There are textual occurrences");
  let repeated = text.querySelector(".file.query-repeated #line-8");
  ok(repeated, "including line 8, which a use already shows");
  ok(repeated.querySelector("mark.query-hit")?.textContent == "Dispatcher",
     "for the match in `aDispatcher`");
  ok(!TestUtils.isShown(repeated.closest(".file")), "which is hidden");
  let checkbox = text.querySelector(".query-repeated-toggle input");
  ok(checkbox && !checkbox.checked, "with an unchecked checkbox");
  ok(/Include \d+ lines? already shown by other results/.test(checkbox.closest("label").textContent),
     "which counts them");

  checkbox.click();
  ok(TestUtils.isShown(repeated.closest(".file")), "Checking it shows them");
  ok(text.open, "without collapsing the textual occurrences");

  await TestUtils.loadQuery("tests", "Dispatcher");
  text = textualOccurrences();
  checkbox = text.querySelector(".query-repeated-toggle input");
  ok(checkbox.checked, "It's remembered");
  ok(TestUtils.isShown(text.querySelector(".file.query-repeated")), "as is what it shows");
});
