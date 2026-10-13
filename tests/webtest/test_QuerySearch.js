"use strict";

// The trydelta tree searches with the pipeline-server's /query/ (its config's
// "search_endpoint": "query"; see `config::SearchEndpoint`): its pages' search
// box is a single field submitted with Enter, and their links to searches go
// to /query/default.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_QuerySearchBox() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const form = doc().querySelector("#search-box");
  is(form.getAttribute("action"), "/trydelta/query/default",
     "A source listing's search box searches with /query/");
  ok(!doc().querySelector("#path"), "with no path filter");
  ok(!doc().querySelector("#case") && !doc().querySelector("#regexp"),
     "nor case and regexp options");
  is(doc().querySelector("#data").getAttribute("data-search"), "/trydelta/query/default",
     "and the page's scripts search there too");

  // (Pressing Enter submits the form.)
  doc().querySelector("#query").value = "Frob";
  const loaded = TestUtils.waitForLoad();
  form.requestSubmit();
  await loaded;
  is(doc().location.pathname, "/trydelta/query/default", "Submitting it searches /query/");
  is(new URLSearchParams(doc().location.search).get("q"), "Frob", "for the query");
  ok(doc().querySelector('details[data-path="widget.h"]'), "with its results");

  await TestUtils.loadPath("/trydelta/source/");
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/query/default",
     "A directory listing's search box searches with /query/");
  ok(!doc().querySelector("#path"), "with no path filter");
});

add_task(async function test_QuerySearchLinks() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const frob = [...doc().querySelectorAll("#line-8 .source-line span[data-symbols]")]
    .find(span => span.textContent == "Frob");
  TestUtils.click(frob);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for symbol click");

  const search = menu.querySelector(".icon-search")?.getAttribute("href");
  ok(search?.startsWith("/trydelta/query/default?q=symbol:"),
     "Searching for a symbol from the context menu uses /query/");
  ok(!search.includes("redirect="), "without /search's redirect parameter");

  await TestUtils.loadPath(search);
  const foundIn = doc().querySelector(".result-context a")?.getAttribute("href");
  ok(foundIn?.startsWith("/trydelta/query/default?q=symbol:"),
     "and the results' links to what contains them use it too");
});
