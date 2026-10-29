"use strict";

// The trydelta tree searches with the pipeline-server's /query/ (its config's
// "search_endpoint": "query"; see `config::SearchEndpoint`): its pages' search
// box is a single field submitted with Enter, to /search, which normalizes
// what's typed into a query (see the tools' `query::normalize`), whose page
// says where a DWIM rule says it may be (`query::dwim`), and their links to
// searches go to /query/default.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_QuerySearchBox() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const form = doc().querySelector("#search-box");
  is(form.getAttribute("action"), "/trydelta/search",
     "A source listing's search box sends what's typed to /search");
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
  is(doc().querySelector("#search-box").getAttribute("action"), "/trydelta/search",
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

  // What contains the results ("// found in X") is a symbol, whose context
  // menu is a symbol's too, searching with /query/.
  // (A function's, rather than a namespace's, which has no definition.)
  await TestUtils.loadPath(search);
  const foundIn = [...doc().querySelectorAll(".result-context [data-symbols]")]
    .find(span => span.textContent == "trydelta::FrobTwice");
  ok(foundIn && frame.contentWindow.SYM_INFO[foundIn.dataset.symbols],
     "What contains the results is a symbol, whose info the page has");
  TestUtils.click(foundIn);
  const contextMenu = doc().querySelector("#context-menu");
  await waitForShown(contextMenu, "Context menu is shown for its click");
  const links = [...contextMenu.querySelectorAll("a")];
  ok(links.some(a => a.textContent.includes("Go to definition of")),
     "which goes to its definition");
  ok(contextMenu.querySelector(".icon-search")?.getAttribute("href")
       ?.startsWith("/trydelta/query/default?q=symbol:"),
     "and searches for it with /query/");
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
  // (Which /search would have made a phrase.)
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
  is(params.get("q"), "'Widget::Frob' path:widget case:sensitive",
     "with its path filter and case-sensitivity");
});

add_task(async function test_DwimRules() {
  // A search for a URL of a file elsewhere (ex: GitHub's) is of the URL, and
  // goes to the tree's file if it finds nothing.
  const github = "https://github.com/mozilla-firefox/firefox/blob/main/user.cpp#L8";
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(github)}`);
  is(doc().location.pathname, "/trydelta/source/user.cpp",
     "A search for a GitHub URL of a file, which finds nothing, goes to the file");
  is(doc().location.hash, "#8", "at its line");

  // A path the tree doesn't have is only searched for.
  const missing = "https://github.com/mozilla-firefox/firefox/blob/main/missing.cpp";
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(missing)}`);
  is(doc().location.pathname, "/trydelta/query/default", "One of a file the tree doesn't have");
  is(new URLSearchParams(doc().location.search).get("q"), `'${missing}'`, "is searched for");
  ok(!doc().querySelector(".query-dwim"), "without where it is");

  // A search for a URL which finds it (ex: code citing it) shows what it
  // finds, and says where the URL's file is (the page of a search with its
  // input, in any tree; see tests/tests/files/spec_excerpts.cpp's end).
  const cited = "https://github.com/example/tests/blob/main/spec_excerpts.cpp";
  await TestUtils.loadQuery("tests", `'${cited}'`, { input: cited });
  is(doc().location.pathname, "/tests/query/default", "A search for a URL which finds it stays");
  ok(doc().querySelector('details[data-path="spec_excerpts.cpp"]'), "with what it finds");
  is(doc().querySelector(".query-dwim a")?.getAttribute("href"), "/tests/source/spec_excerpts.cpp",
     "and says where its file is");
  // (But not a query's, which isn't a person's search.)
  await TestUtils.loadQuery("tests", `'${cited}'`);
  ok(!doc().querySelector(".query-dwim"), "A query doesn't");
});

add_task(async function test_QuerySearchGoesAtOnce() {
  // A DWIM rule with `go = "always"` (ex: a web spec's URL's) goes to its
  // target at once, without searching, whose page says what was searched for
  // and offers its search (see search.js's showSearched).  (trydelta's specs
  // are the tests tree's, its `SPEC_` linked tree.)
  const bs = "https://example.github.io/bs/#poke";
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(bs)}`);
  is(doc().location.pathname, "/tests/rendered/specs/example.bs",
     "A Bikeshed spec's URL goes to its rendered view, in the tree with its specs");
  is(doc().location.hash, "#poke", "at its anchor");
  is(doc().querySelector(".dwim-searched code")?.textContent, bs, "which says what was searched for");
  const search = new URL(doc().querySelector(".dwim-searched a").href);
  is(search.pathname, "/trydelta/query/default", "and offers the search, in the tree searched");
  is(search.searchParams.get("q"), `'${bs}'`, "for the URL");
  ok(!doc().location.search.includes("searched"), "and its URL doesn't say so");

  // (A paged spec's anchor goes to its page, saying so too.)
  const wattsi = "https://example.spec.whatwg.org/#assemble";
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent(wattsi)}`);
  is(doc().location.pathname, "/tests/rendered/specs/source/gadgets.html",
     "A paged spec's URL goes to the page with its anchor");
  is(doc().location.hash, "#assemble", "at it");
  is(doc().querySelector(".dwim-searched code")?.textContent, wattsi, "saying what was searched for");
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
  ok(doc().body.textContent.includes("Only path:, pathre: and tree: terms can be negated"),
     "Negating another term says it can't be");
});

add_task(async function test_QueryCaseSensitivity() {
  // A query's searches of text and identifiers ignore case, but with
  // `case:sensitive` (which router.py's search's "Case-sensitive" option is).
  await TestUtils.loadQuery("tests", "text:AWIDGET");
  ok(doc().querySelector('details[data-path="spec_citations.cpp"]'), "A search ignores case");
  await TestUtils.loadQuery("tests", "text:AWIDGET case:sensitive");
  ok(!doc().querySelector('details[data-path="spec_citations.cpp"]'),
     "but not with case:sensitive");
  await TestUtils.loadQuery("tests", "text:aWidget case:sensitive");
  ok(doc().querySelector('details[data-path="spec_citations.cpp"]'),
     "which finds the case it has");
});
