"use strict";

// The trydelta tree's try pushes get semantic analysis (see
// tests/trydelta/import-try and tools/src/try_delta.rs): 0123456789ab changes
// Widget::Frob's signature (which user.cpp's call and gadget.h's then use,
// though they don't change), adds Widget::Wobble, and adds wobbler.cpp, which
// calls it.

const PUSH = "0123456789ab";
const NEW_FROB = "_ZN8trydelta6Widget4FrobEib";
const OLD_FROB = "_ZN8trydelta6Widget4FrobEi";

function doc() {
  return frame.contentDocument;
}

// Import the push (if it isn't yet), and return the head of its stack rebased
// onto the indexed revision, from its page's link to its semantic analysis.
async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/${PUSH}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  const link = doc().querySelector(".explore-semantic-head a")?.getAttribute("href");
  ok(link?.startsWith("/trydelta/rev/"), "The push's page links to its semantic analysis");
  return link.split("/")[3];
}

add_task(async function test_TryDeltaPages() {
  const head = await importPush();
  is(doc().querySelector("#data").getAttribute("data-search-rev"), head,
     "and searches from it are of the push");

  await TestUtils.loadPath(`/trydelta/rev/${head}/user.cpp`);
  is(doc().querySelector("#data").getAttribute("data-search-rev"), head,
     "The head's pages search in the push's context");
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/query/default",
     "with /query/");
  is(doc().querySelector('#search-box input[name="rev"]')?.value, head, "for the push");

  const frob = [...doc().querySelectorAll("#line-8 .source-line span[data-symbols]")]
    .find(span => span.textContent == "Frob");
  ok(frob?.getAttribute("data-symbols").split(",").includes(NEW_FROB),
     "An unchanged file's call has the symbol of the signature the push gives it");
  ok(frame.contentWindow.SYM_INFO[NEW_FROB]?.jumps?.def?.startsWith("widget.cpp"),
     "which jumps to the push's definition");

  TestUtils.click(frob);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");
  const search = menu.querySelector(".icon-search")?.getAttribute("href");
  ok(search?.startsWith("/trydelta/query/default?q=symbol:") && search.endsWith(`&rev=${head}`),
     "and its searches are of the push");
});

add_task(async function test_TryDeltaQueries() {
  const head = await importPush();

  await TestUtils.loadPath(`/trydelta/query/default?q=Wobble&rev=${head}`);
  ok(doc().querySelector(".query-try-context")?.textContent.includes(PUSH),
     "A query in the push's context says so");
  ok(doc().querySelector(`a[href="/trydelta/rev/${head}/wobbler.cpp"]`),
     "and finds the files the push adds, at its head");
  ok(doc().querySelector(`a.query-line-number[href^="/trydelta/rev/${head}/"]`),
     "whose lines link to its head's");
  is(doc().querySelector('#search-box input[name="rev"]')?.value, head,
     "Searching again stays in the push's context");
  await TestUtils.loadPath("/trydelta/query/default?q=Wobble");
  ok(!doc().querySelector('a[href$="/wobbler.cpp"]'), "The indexed revision doesn't have them");

  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${NEW_FROB}&rev=${head}`);
  ok(doc().querySelector('a[href$="/user.cpp"]'),
     "The push's new symbols' uses include those of files it doesn't change");
  ok(doc().querySelector('a[href$="/gadget.h"]'), "headers included");
  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${OLD_FROB}&rev=${head}`);
  ok(!doc().querySelector('a[href$="/user.cpp"]'), "and the symbols it replaces have none");
  await TestUtils.loadPath(`/trydelta/query/default?q=symbol:${OLD_FROB}`);
  ok(doc().querySelector('a[href$="/user.cpp"]'), "unlike in the indexed revision");
});
