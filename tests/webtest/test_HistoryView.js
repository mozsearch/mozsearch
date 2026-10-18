"use strict";

// History pages (see `format::history_data`): the history of a scope of a
// file (a context of the history's, ex: a function, or the whole file), from
// the token menu, as an explore page of the commits which changed it.  We use
// the fixed revision of blame.rs that test_TokenBlame.js uses, whose line 12
// is in find_phab_rev, which bug 1975601's Part 1 added and a clippy fix
// changed.
const REV = "50df75fc5253890347bfac70037bc7ad42316862";
const PATH = `/searchfox/rev/${REV}/tools/src/blame.rs`;

function doc() {
  return frame.contentDocument;
}

function summaries() {
  return [...doc().querySelectorAll(".explore-commit-summary")].map(s => s.textContent);
}

add_task(async function test_HistoryView() {
  await TestUtils.loadPath(PATH);
  const strip = doc().querySelector("#line-12 .blame-strip");
  const popup = doc().querySelector("#blame-popup");
  frame.contentWindow.BlamePopup.triggerElement = null;
  TestUtils.dispatchMouseEvent("mouseenter", strip);
  await waitForCondition(
    () => popup.style.display != "none" && frame.contentWindow.BlamePopup.popupOwner == strip,
    "Line 12's blame popup is shown");
  const token = [...popup.querySelectorAll(".hb-replica .hb-token")].find(
    t => t.textContent == "strip_prefix");
  TestUtils.click(token);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "A token's menu is shown");
  const item = [...menu.querySelectorAll("a")].find(
    a => a.textContent == "Show the history of this token's method, class, or file");
  ok(item, "which offers the history of where the token is");

  await TestUtils.loadPath(item.getAttribute("href"));
  is(doc().querySelector(".explore-page h2")?.textContent, "History of find_phab_rev",
     "whose page is the history of the token's function");
  const own = summaries();
  ok(own[0]?.includes("Part 1: Show the phabricator revision"),
     "starting with the commit which added it");
  ok(own.some(s => s.includes("cargo clippy --fix")), "with the clippy fix which changed it");
  ok(!own.some(s => s.includes("Bug 1282179")),
     "but not the file's commits from before the function");
  ok(doc().querySelector('.explore-symbol .explore-name')?.textContent.includes("find_phab_rev"),
     "Its sparkline is the function's");
  const scopes = [...doc().querySelectorAll(".explore-scopes a")];
  is(scopes[0]?.textContent, "tools/src/blame.rs", "It links to the history of the whole file");

  await TestUtils.loadPath(scopes[0].getAttribute("href"));
  is(doc().querySelector(".explore-page h2")?.textContent, "History of tools/src/blame.rs",
     "whose page is the file's history");
  const file = summaries();
  ok(file.length > own.length, "with more commits");
  ok(file.some(s => s.includes("Bug 1282179")), "including those from before the function");
});

// A scope which moved to its file from another file (ex: the result facets'
// MaybeFacetRoot, which Part 120 moved from cmd_compile_results.rs to
// facets.rs) has its history in the other file before the move, which the
// sparklines of both files' versions of it note.
add_task(async function test_HistoryAcrossMoves() {
  const MOVE = "4d987207087ae51e98792a49fe36e6aaff8a7ca2";
  const OLD = "tools/src/cmd_pipeline/cmd_compile_results.rs";
  const NEW = "tools/src/cmd_pipeline/facets.rs";
  await TestUtils.loadPath(`/searchfox/history/${MOVE}/${NEW}?scope=MaybeFacetRoot`);
  ok(doc().querySelector(".explore-note")?.textContent.includes(`MaybeFacetRoot moved from ${OLD}`),
     "The history of a symbol which moved to its file says where it was");
  const own = summaries();
  ok(own[0]?.includes("Implement `compile-results`"),
     "and starts with the commit which added it there");
  ok(own[own.length - 1]?.includes("Part 120"), "up to the commit which moved it");
  const paths = [...doc().querySelectorAll(".explore-file")].map(file => file.dataset.path);
  ok(paths.includes(OLD) && paths.includes(NEW), "Its changes are in both files");

  const from = doc().querySelector(`.explore-file[data-path="${NEW}"] .explore-move`);
  is(from?.textContent, "↤ cmd_compile_results.rs", "Its sparkline says where it moved from");
  ok(from?.getAttribute("href").endsWith(`/${OLD}`) && !from.getAttribute("href").includes(MOVE),
     "linking to that file, before the move");
  const to = doc().querySelector(`.explore-file[data-path="${OLD}"] .explore-move`);
  is(to?.textContent, "↦ facets.rs", "and its old self's says where it moved to");
});
