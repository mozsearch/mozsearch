"use strict";

// A link in a source listing (a URL or a bug number in a comment) shows the
// context menu instead of leaving the page, with an item to follow it first,
// and searches for it.

// Click the link, saying whether the menu prevented following it (after which
// the test prevents it either way, so as not to leave the page).
function clickLink(link, options = {}) {
  let prevented = null;
  frame.contentWindow.addEventListener("click", event => {
    prevented = event.defaultPrevented;
    event.preventDefault();
  }, { once: true });
  link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ...options }));
  return prevented;
}

async function menuForLink(path, selector) {
  await TestUtils.loadPath(path);
  const link = frame.contentDocument.querySelector(selector);
  is(link?.tagName, "A", "The link is a link");

  ok(clickLink(link), "Clicking the link doesn't follow it");
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the link");
  is(frame.contentDocument.location.pathname, path, "The page stays");
  return [...menu.querySelectorAll("a")];
}

add_task(async function test_SpecCitationLink() {
  const url = "https://example.spec.whatwg.org/#concept-widget-poke";
  const items = await menuForLink(
    "/tests/source/spec_citations.cpp", "#line-11 code.source-line a[href]");

  is(items[0].textContent, `Follow link to ${url}`, "The first item follows the link");
  is(items[0].getAttribute("href"), url, "to its URL");
  ok(items.find(a => a.textContent.includes("Go to definition of example poke")),
     "The citation's symbol's items remain");

  const searches = items
    .filter(a => a.textContent.startsWith("Search for the substring"))
    .map(a => a.textContent);
  is(JSON.stringify(searches), JSON.stringify([
    `Search for the substring ${url}`,
    "Search for the substring https://example.spec.whatwg.org/",
  ]), "Its searches are for the URL, and the URL without its fragment");
});

add_task(async function test_BugLink() {
  const url = "https://bugzilla.mozilla.org/show_bug.cgi?id=2063622";
  const items = await menuForLink(
    "/tests/source/bug2063622.cpp", "#line-1 code.source-line a[href]");

  is(items[0].getAttribute("href"), url, "The first item follows the link");
  ok(items.find(a => a.textContent == "Search for the substring 2063622"),
     "Its search is for the bug's number");
});

add_task(async function test_ModifiedClickFollowsLink() {
  await TestUtils.loadPath("/tests/source/bug2063622.cpp");
  const link = frame.contentDocument.querySelector("#line-1 code.source-line a[href]");

  ok(!clickLink(link, { ctrlKey: true }), "A Ctrl+click is the browser's");
  is(frame.contentDocument.querySelector("#context-menu")?.style.display ?? "none", "none",
     "with no menu");
});
