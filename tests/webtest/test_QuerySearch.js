"use strict";

// The trydelta tree searches with the pipeline-server's /query/ (its config's
// "search_endpoint": "query"; see `config::SearchEndpoint`): its pages' search
// box is a single field submitted with Enter, to the DWIM endpoint, which
// normalizes what's typed into a query (see the tools' `query::dwim`), and
// their links to searches go to /query/default.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_QuerySearchBox() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const form = doc().querySelector("#search-box");
  is(form.getAttribute("action"), "/trydelta/dwim",
     "A source listing's search box sends what's typed to the DWIM endpoint");
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
  const params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "Frob", "for the query");
  ok(!params.has("input"), "which is what was typed");
  ok(!doc().querySelector(".query-interpretation"), "so the page doesn't say how it read it");
  ok(doc().querySelector('details[data-path="widget.h"]'), "with its results");

  await TestUtils.loadPath("/trydelta/source/");
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/dwim",
     "A directory listing's search box does too");
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

add_task(async function test_QueryNormalization() {
  // A qualified name, which the query language would take as the term
  // `Widget`, is quoted.
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  doc().querySelector("#query").value = "Widget::Frob";
  let loaded = TestUtils.waitForLoad();
  doc().querySelector("#search-box").requestSubmit();
  await loaded;
  is(doc().location.pathname, "/trydelta/query/default", "Searching for a qualified name queries");
  let params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "'Widget::Frob'", "for it quoted");
  is(params.get("input"), "Widget::Frob", "with what was typed");
  const interpretation = doc().querySelector(".query-interpretation")?.textContent ?? "";
  ok(interpretation.includes("Widget::Frob") && interpretation.includes("quoted the qualified name"),
     `which the page says it read as the query: ${interpretation.trim()}`);
  is(doc().querySelector("#query").value, "'Widget::Frob'", "and its search box has the query");
  ok(doc().querySelector('details[data-path="widget.cpp"]'), "with its results");

  // A query's page's box (ex: a diagram's) has the query language, and
  // searches with /query/ as it is, rather than the DWIM endpoint.
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/query/default",
     "A query's page's box searches with /query/");
  loaded = TestUtils.waitForLoad();
  doc().querySelector("#search-box").requestSubmit();
  await loaded;
  params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "'Widget::Frob'", "Searching for the query again");
  ok(!params.has("input"), "is of what was typed");
  // (Which the DWIM endpoint would have made a phrase.)
  doc().querySelector("#query").value = "Widget Frob";
  loaded = TestUtils.waitForLoad();
  doc().querySelector("#search-box").requestSubmit();
  await loaded;
  params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "Widget Frob", "and searches for a query as it's typed");
  ok(!params.has("input"), "without reading it as a person's search");

  // router.py's search's URLs (ex: old links and keyword bookmarks), with its
  // form's fields, are its query.
  await TestUtils.loadPath("/trydelta/search?q=Widget::Frob&path=widget&case=true&regexp=false");
  is(doc().location.pathname, "/trydelta/query/default", "A search's URL queries");
  params = new URLSearchParams(doc().location.search);
  is(params.get("q"), "'Widget::Frob' path:widget", "with its path filter");
  ok([...doc().querySelectorAll(".query-interpretation")].some(
       div => div.textContent.includes("case-sensitively")),
     "saying that queries can't search case-sensitively");
});

add_task(async function test_DwimRules() {
  // A URL of a file elsewhere (ex: GitHub's) goes to the tree's.
  const github = "https://github.com/mozilla-firefox/firefox/blob/main/user.cpp#L8";
  await TestUtils.loadPath(`/trydelta/dwim?q=${encodeURIComponent(github)}`);
  is(doc().location.pathname, "/trydelta/source/user.cpp", "A GitHub file's URL goes to the file");
  is(doc().location.hash, "#8", "at its line");

  // A path the tree doesn't have is searched for.
  const missing = "https://github.com/mozilla-firefox/firefox/blob/main/missing.cpp";
  await TestUtils.loadPath(`/trydelta/dwim?q=${encodeURIComponent(missing)}`);
  is(doc().location.pathname, "/trydelta/query/default", "One of a file the tree doesn't have");
  is(new URLSearchParams(doc().location.search).get("q"), `'${missing}'`, "is searched for");

  // In a tree searching with router.py's search, it has what isn't a rule's.
  await TestUtils.loadPath(`/tests/dwim?q=${encodeURIComponent("Foo::Bar baz")}`);
  is(doc().location.pathname, "/tests/search", "Other trees' DWIM searches");
  is(new URLSearchParams(doc().location.search).get("q"), "Foo::Bar baz", "for what was typed");
});

add_task(async function test_QueryExcludedPaths() {
  // `-path:` (and `-pathre:`) leaves out paths from the files and text a query
  // searches.
  await TestUtils.loadQuery("tests", "aWidget");
  ok(doc().querySelector('details[data-path="spec_excerpts.cpp"]') &&
     doc().querySelector('details[data-path="spec_citations.cpp"]'),
     "A search has its files' lines");
  await TestUtils.loadQuery("tests", "aWidget -path:spec_excerpts.cpp");
  ok(!doc().querySelector('details[data-path="spec_excerpts.cpp"]'),
     "but not those of a path it leaves out");
  ok(doc().querySelector('details[data-path="spec_citations.cpp"]'), "but others'");

  // (Other terms can't be negated; a negated word is text, with its dash.)
  await TestUtils.loadQuery("tests", "aWidget -calls-to:Poke");
  ok(doc().body.textContent.includes("Only path: and pathre: terms can be negated"),
     "Negating another term says it can't be");
});
