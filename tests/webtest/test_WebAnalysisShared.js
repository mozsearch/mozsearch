"use strict";

// The trydelta-shared tree's web analyses (its scip-analyze, js-environments-
// analyze, js-, html- and css-analyze, js-environments-pages and
// document-links steps') are the trydelta tree's (`web_analysis_from`; see
// scripts/web-analysis-share.sh), the same revision's, whose build has the
// C++ and Rust its doesn't.  (If they weren't, its indexing would warn, which
// fails the webtests' indexing.)

const FILES = [
  "hello.js",
  "testing/web-platform/tests/greeting/greeting.html",
  "testing/web-platform/tests/greeting/support/greeting.css",
  "testing/web-platform/tests/resources/testharness.js",
];

async function rawAnalysis(tree, path) {
  const response = await fetch(`/${tree}/raw-analysis/${path}`, { cache: "no-store" });
  return { status: response.status, text: await response.text() };
}

add_task(async function test_WebAnalysisShared() {
  for (const path of FILES) {
    const theirs = await rawAnalysis("trydelta", path);
    const shared = await rawAnalysis("trydelta-shared", path);
    is(theirs.status, 200, `trydelta has ${path}'s analysis`);
    ok(theirs.text.includes('"sym"'), `with its symbols`);
    is(shared.status, 200, `trydelta-shared has ${path}'s analysis`);
    is(shared.text, theirs.text, "the same");
  }
  // (Its build's analyses are its own.)
  is((await rawAnalysis("trydelta", "widget.cpp")).status, 200,
     "trydelta has its C++'s analysis");
  is((await rawAnalysis("trydelta-shared", "widget.cpp")).status, 404,
     "trydelta-shared, which doesn't build it, doesn't");
});
