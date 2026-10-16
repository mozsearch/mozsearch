"use strict";

// A web spec's terms' pretty names are "<spec> <term>" (with "/"-separated
// "for"s, ex: "html Navigation/navigate(url, options)"), whose terms are
// identifiers too (see `split_pretty`), so that searching for a term finds its
// definition: the tests tree's example spec's "poke" algorithm's is
// "example poke".
add_task(async function test_SpecTermQuery() {
  await TestUtils.loadQuery("tests", "poke");
  const content = frame.contentDocument.querySelector("#content");
  ok(content.textContent.includes("Definitions (example poke)"),
     "/query/ finds a spec term's definition by the term");
});

// (The router reads identifiers with spaces too.)
add_task(async function test_SpecTermSearch() {
  await TestUtils.loadPath("/tests/search?q=poke");
  const content = frame.contentDocument.querySelector("#content");
  await waitForCondition(() => content.textContent.includes("Definitions (example poke)"),
                         "Classic search finds a spec term's definition by the term");
  await TestUtils.loadPath(`/tests/search?q=${encodeURIComponent("example poke")}`);
  await waitForCondition(() => frame.contentDocument.querySelector("#content").textContent.includes("Definitions (example poke)"),
                         "and by its whole pretty name");
});
