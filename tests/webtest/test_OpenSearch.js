"use strict";

// The trees' OpenSearch descriptions (/{tree}/opensearch.xml, which their
// pages link to): installed as the browser's search engines (in its own
// context; see TestHarness.chrome), they search with /search (which in a tree
// searching with queries normalizes what's searched for into one; see the
// tools' `query::normalize`), and suggest identifiers, which the address bar
// shows.

function doc() {
  return frame.contentDocument;
}

// (Install the tree's engine, run `script` (an async function's body, given
// `engine`, `SearchService` and `SearchSuggestionController`) with it, and
// remove it.)
function withEngine(tree, script) {
  return TestHarness.chrome(`
    const [url, done] = arguments;
    (async () => {
      const { SearchService } = ChromeUtils.importESModule(
        "moz-src:///toolkit/components/search/SearchService.sys.mjs");
      const { SearchSuggestionController } = ChromeUtils.importESModule(
        "moz-src:///toolkit/components/search/SearchSuggestionController.sys.mjs");
      await SearchService.init();
      const engine = await SearchService.addOpenSearchEngine(url, null);
      try {
        return await (async () => { ${script} })();
      } finally {
        await SearchService.removeEngine(engine);
      }
    })().then(done, error => done({ error: String(error) }));
  `, `${location.origin}/${tree}/opensearch.xml`);
}

add_task(async function test_OpenSearchDescription() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const link = doc().querySelector('link[rel="search"]');
  is(link?.getAttribute("href"), "/trydelta/opensearch.xml",
     "A page links to its tree's OpenSearch description");
  is(link?.getAttribute("title"), "Searchfox (trydelta)", "titled with its engine's name");

  const response = await fetch("/trydelta/opensearch.xml", { cache: "no-store" });
  const xml = new DOMParser().parseFromString(await response.text(), "application/xml");
  is(xml.querySelector("ShortName")?.textContent, "Searchfox (trydelta)",
     "whose name is the link's title");
  const templates = Object.fromEntries([...xml.querySelectorAll("Url")].map(
    url => [url.getAttribute("type"), url.getAttribute("template")]));
  is(templates["text/html"], `${location.origin}/trydelta/search?q={searchTerms}`,
     "It searches with /search (by its absolute URL)");
  is(templates["application/x-suggestions+json"],
     `${location.origin}/trydelta/suggest?q={searchTerms}`, "and has suggestions");
});

add_task(async function test_Suggestions() {
  const suggest = async (tree, q) => (await fetch(`/${tree}/suggest?q=${encodeURIComponent(q)}`,
                                                  { cache: "no-store" })).json();
  const [input, suggestions] = await suggest("trydelta", "Widget::Fr");
  is(input, "Widget::Fr", "Suggestions are of the input");
  ok(suggestions.includes("Widget::Frob "),
     `the identifiers it's the prefix of, with a space (or the address bar leaves them out): ${JSON.stringify(suggestions)}`);
  // (Which router.py's search would search for, so not in its trees.)
  const [, router] = await suggest("tests", "outerNS::Th");
  ok(router.includes("outerNS::Thing"), `A tree searching with router.py's search's don't: ${JSON.stringify(router)}`);

  const [, none] = await (await fetch("/tests/suggest?q=path%3Afoo%20bar", { cache: "no-store" })).json();
  is(none.length, 0, "Input of more than a word has none");
});

add_task(async function test_OpenSearchEngine() {
  const result = await withEngine("trydelta", `
    const submission = engine.getSubmission("Widget::Frob").uri.spec;
    const controller = new SearchSuggestionController();
    const fetched = await controller.fetch({
      searchString: "Widget::Fr", inPrivateBrowsing: false, engine,
    });
    return {
      name: engine.name,
      submission,
      suggestions: fetched.remote.map(entry => entry.value),
    };
  `);
  is(result.name, "Searchfox (trydelta)", "The tree's engine installs, with its name");
  ok(result.submission.startsWith(`${location.origin}/trydelta/search?q=Widget%3A%3AFrob`),
     `and searches with /search: ${result.submission}`);
  ok(result.suggestions.includes("Widget::Frob "),
     `It has suggestions: ${JSON.stringify(result.suggestions)}`);
});

add_task(async function test_AddressBarSuggestions() {
  // (In the engine's search mode, as after typing its keyword; the address
  // bar may restart its query on entering it.)
  const result = await withEngine("trydelta", `
    const window = Services.wm.getMostRecentWindow("navigator:browser");
    const urlbar = window.gURLBar;
    urlbar.search("Widget::Fr", { searchEngine: engine, searchModeEntry: "other" });
    let suggestions = [];
    for (let i = 0; i < 20 && !suggestions.length; i++) {
      const context = await urlbar.lastQueryContextPromise;
      suggestions = context.results
        .map(result => result.payload.suggestion)
        .filter(suggestion => suggestion);
      if (!suggestions.length) {
        await new Promise(resolve => window.setTimeout(resolve, 100));
      }
    }
    urlbar.view.close();
    urlbar.handleRevert();
    return suggestions;
  `);
  ok(result.includes("Widget::Frob "),
     `The address bar shows the engine's suggestions: ${JSON.stringify(result)}`);
});
