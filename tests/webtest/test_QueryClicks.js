"use strict";

// /query/'s results link their line numbers to the lines (rather than
// selecting them as in a source listing), give their symbols the source
// listings' context menu, and only toggle their groups from the disclosure
// triangles, the files' icons, and the kinds' names.

add_task(async function test_QueryLineNumbers() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  const link = doc.querySelector(".query-line-number");
  ok(link, "The line numbers are links");
  const path = link.closest("details").querySelector("summary h3.path").textContent.trim();
  ok(link.href.endsWith(`/tests/source/${path}#${link.textContent}`), "to the lines in the files");
  ok(!/\s\/|\/\s/.test(path), "The paths have no spaces around their separators");
});

add_task(async function test_QuerySymbolMenu() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  const token = [...doc.querySelectorAll(".query-result code.source-line span[data-symbols]")]
    .find(span => span.textContent == "doublePure");
  ok(token, "The excerpts have symbols");
  TestUtils.click(token);
  const menu = doc.querySelector("#context-menu");
  await waitForShown(menu, "Clicking a symbol shows the context menu");
  ok(menu.textContent.includes("Go to definition of"),
     "with the source listings' items, since the page has the symbols' information");
});

add_task(async function test_QueryToggles() {
  await TestUtils.loadQuery("tests", "doublePure");

  const doc = frame.contentDocument;
  // (Cancelable, like real clicks, since query-results.js cancels them, and
  // with a click count, since it lets keyboard activation through.)
  const click = (elem, position = {}) =>
    TestUtils.click(elem, { bubbles: true, cancelable: true, detail: 1, ...position });
  // The rest of a summary's row beside its heading is the summary itself, as is
  // its triangle (the summary's marker), so these click its middle at its
  // right edge and at its left edge.
  const rowEnd = summary => {
    const rect = summary.getBoundingClientRect();
    return { clientX: rect.right - 2, clientY: rect.top + rect.height / 2 };
  };
  const triangle = summary => {
    const rect = summary.getBoundingClientRect();
    return { clientX: rect.left + 2, clientY: rect.top + rect.height / 2 };
  };
  const file = [...doc.querySelectorAll(".query-result details")]
    .find(details => details.querySelector(":scope > summary h3.path"));
  const path = file.querySelector(":scope > summary h3.path");
  click(path);
  ok(file.open, "Clicking a file's path row doesn't collapse it");
  click(path.querySelector(".mimetype-bullet"));
  ok(!file.open, "Clicking its icon does");
  click(path.querySelector(".mimetype-bullet"));
  const fileSummary = file.querySelector(":scope > summary");
  click(fileSummary, rowEnd(fileSummary));
  ok(file.open, "Clicking the row to the right of its path doesn't");
  click(fileSummary, triangle(fileSummary));
  ok(!file.open, "Clicking its triangle does");
  click(fileSummary, triangle(fileSummary));

  const kind = [...doc.querySelectorAll(".query-result details")]
    .find(details => details.querySelector(":scope > summary h2"));
  click(kind.querySelector(":scope > summary h2"));
  ok(kind.open, "Clicking a kind's heading doesn't collapse it");
  const kindSummary = kind.querySelector(":scope > summary");
  click(kindSummary, rowEnd(kindSummary));
  ok(kind.open, "Clicking the row to the right of its heading doesn't");
  click(kind.querySelector(":scope > summary .query-toggle"));
  ok(!kind.open, "Clicking the kind's name does");
  click(kindSummary, triangle(kindSummary));
  ok(kind.open, "Clicking its triangle does");
});
