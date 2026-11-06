"use strict";

// The tree's markup-like text is text in every page showing it, never
// markup.

function injected(id) {
  return frame.contentDocument.getElementById(id);
}

add_task(async function test_SourceListing() {
  // (The tree's tests/files/xss/.)
  await TestUtils.loadPath("/tests/source/xss/injections.js");
  ok(!injected("xss-sym-info"), "A string's </script> doesn't end the page's JSON data");
  ok(typeof frame.contentWindow.SYM_INFO == "object", "which the page has");
  ok(!frame.contentDocument.querySelector('a[href^="javascript"]'),
     "A comment's javascript:// URL isn't a link");
  ok(!frame.contentDocument.querySelector("[data-xss-moz-src]"),
     "A comment's moz-src URL's quote doesn't end its attribute");

  await TestUtils.loadPath("/tests/source/xss/tokens.html");
  ok(!injected("xss-newline"), "A line's pending text is escaped");
});

add_task(async function test_SearchResults() {
  // (The router's results, which search.js renders.)
  await TestUtils.loadPath("/tests/search?q=xssTarget&redirect=false");
  await TestUtils.waitForCondition(
    () => frame.contentDocument.querySelector(".result-context"),
    "The search results have the use's context"
  );
  ok(!injected("xss-context"), "A key that's a use's context is text");
  ok(frame.contentDocument.querySelector(".result-context").textContent.includes("<img id=xss-context"),
     "which is shown as it is");

  // (The query page's, which its templates render.)
  await TestUtils.loadQuery("tests", "xssTarget");
  await TestUtils.waitForCondition(
    () => frame.contentDocument.querySelector(".result-context"),
    "The query results have the use's context"
  );
  ok(!injected("xss-context"), "and so does the query page's");
});
