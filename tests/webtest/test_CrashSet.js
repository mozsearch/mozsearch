"use strict";

// A crash set's view (the `crash-set:` query term's; see
// tools/src/crash_sets.rs): the crash reports a crash-stats search found,
// fetched once and stored, grouped into families by what their threads other
// than the crashing one were busy with in the tree's code (but what most of
// them were busy with), and variants by their functions; the navigation
// panel's facets of its crashes; and its crashes' views, from the store.  The
// set is tests/crash-sets/'s fixture (see webtest-config.json's
// `crash_sets_path`): 6 crashes of a QuotaManager shutdown timeout, whose
// QuotaManager IO thread was busy in dom/quota in 3 (2 variants), whose
// DOMCacheThread was busy in dom/cache in 2, with nothing else in 1, and
// whose Link Monitor thread was busy in all of them.
const SET = "8487b0f27bafe761";

function doc() {
  return frame.contentDocument;
}

add_task(async function test_CrashSet() {
  await TestUtils.loadQuery("searchfox", `crash-set:${SET}`);
  const notice = doc().querySelector(".crash-set-view .crash-notice")?.textContent || "";
  ok(notice.includes(`The crash set ${SET}: 6 crash reports from crash-stats' search for the signature mozilla::dom::quota::QuotaManager::Shutdown::<T>::operator() (Firefox, nightly, 2026-10-26 to 2026-11-02)`),
     `The view says what the set is: ${notice}`);
  ok(doc().querySelector(".crash-set-search")?.href.startsWith("https://crash-stats.mozilla.org/search/?product=Firefox&signature=%3Dmozilla"),
     "with its search on crash-stats");
  ok(doc().querySelector(".crash-set-common")?.textContent.includes("Link Monitor in netwerk/system/win32 (6)"),
     "and what they all were busy with, which says nothing of a crash");

  const families = [...doc().querySelectorAll(".crash-family")].map(family => [
    family.querySelector(".crash-family-key").textContent,
    family.querySelector(".crash-family-count").textContent,
    [...family.querySelectorAll(".crash-variant-count")].map(count => count.textContent),
  ]);
  is(JSON.stringify(families), JSON.stringify([
    ["QuotaManager IO in dom/quota", "3 crashes", ["2 crashes", "1 crash"]],
    ["DOMCacheThread in dom/cache", "2 crashes", ["2 crashes"]],
    ["nothing else distinctive busy; crashing thread called from xpcom/threads", "1 crash", ["1 crash"]],
  ]), "Its families, the biggest first, and their variants");
  is(doc().querySelector(".crash-variant-label")?.textContent,
     "QuotaManager IO: _PR_MD_FSYNC ‹ mozilla::dom::quota::QuotaManager::UnloadQuota ‹ mozilla::dom::quota::ShutdownStorageOp::DoDirectoryWork",
     "A variant is its busy threads' functions, from their library calls to the tree's code calling them");

  // The diagrams of what they were busy with: a box for each connected part
  // of their stacks' graph (but what they all were busy with), the biggest
  // first, its functions colored and numbered by how many stacks have them.
  const graphs = () => [...doc().querySelectorAll(".crash-graph")].filter(graph => !graph.hidden);
  is(JSON.stringify(graphs().map(graph => graph.querySelector("figcaption").textContent)),
     JSON.stringify(["3 stacks of 3 crashes", "2 stacks of 2 crashes"]), "A diagram for each connected part of their stacks");
  const nodes = graph => [...graph.querySelectorAll(".crash-graph-node")].map(node =>
    [node.querySelector(".crash-graph-node-text").textContent, node.style.getPropertyValue("--heat")]);
  is(JSON.stringify(nodes(graphs()[0])), JSON.stringify([
    ["CreateDirectoryMetadata2 1", "0.333"], ["DoDirectoryWork 3", "1.000"], ["UnloadQuota 2", "0.667"],
  ]), "its functions, without libraries' (NSPR's) and event loops', with their stacks' counts and heat");
  is(graphs()[0].querySelectorAll(".crash-graph-edge").length, 2, "and their calls");
  const top = graphs()[0].querySelector(".crash-graph-node").getBoundingClientRect().top;
  ok([...graphs()[0].querySelectorAll(".crash-graph-node")].some(node => node.getBoundingClientRect().top < top),
     "callers over callees");

  // The navigation panel's facets of its crashes: what they were busy with,
  // by thread, and the directories involved.
  const rows = () => [...doc().querySelectorAll(".crash-set-crash")].filter(row => !row.hidden);
  is(rows().length, 6, "A row per crash");
  is(doc().querySelector(".facet-box")?.dataset.nouns, "crashes", "which the panel's facets filter");
  const facetValue = (facet, name) => [...doc().querySelectorAll(`.facet-box .facet[data-facet="${facet}"] .facet-value`)]
    .find(button => button.textContent.replace(/\s*\d+$/, "") == name);
  ok(facetValue("busy", "QuotaManager IO") && facetValue("busy", "dom/quota"),
     "What they were busy with is by thread, then where");
  ok(facetValue("directory", "dom") && facetValue("directory", "cache"), "and the directories involved, nested");
  const busy = facetValue("busy", "DOMCacheThread");
  TestUtils.click(busy);
  is(rows().length, 2, "Selecting what they were busy with shows those crashes");
  is(new URLSearchParams(doc().location.search).getAll("facet-busy").length, 1, "as the URL says");
  const shownFamilies = [...doc().querySelectorAll(".crash-family")].filter(family => !family.hidden)
    .map(family => [family.querySelector(".crash-family-key").textContent,
                    family.querySelector(".crash-family-count").textContent]);
  is(JSON.stringify(shownFamilies), JSON.stringify([["DOMCacheThread in dom/cache", "2 of 2 crashes"]]),
     "and only the families which have them, with how many of their crashes are shown");
  is(JSON.stringify(graphs().map(graph => graph.querySelector("figcaption").textContent)),
     JSON.stringify(["2 stacks of 2 crashes"]), "and the diagrams which have their stacks");
  const cache = doc().querySelector('.crash-set-crash[data-path$="2611a5"] .crash-set-crash-link');
  is(doc().querySelector(".crash-family:not([hidden]) .crash-variant-latest")?.getAttribute("href"),
     cache.getAttribute("href"), "whose latest crashes are the latest shown");
  TestUtils.click(busy);
  is(rows().length, 6, "Unselecting it shows them all again");
  is(doc().querySelector(".crash-family-count")?.textContent, "3 crashes", "and the families' counts");

  // A crash's view, from the store (crash-stats doesn't have it).
  const latest = doc().querySelector(".crash-variant-latest");
  const url = new URL(latest.href);
  await TestUtils.loadPath(url.pathname + url.search);
  is(doc().querySelector(".crash-crumb-report")?.textContent, "5e4c0b1a-7e57-4c0d-8e5a-0000002611a2",
     "A variant's latest crash's view is the stored crash's");
  is(doc().querySelector("#thread-2 .crash-thread-name")?.textContent, "Thread #2 (QuotaManager IO)",
     "with its threads");
});

add_task(async function test_CrashSetMissing() {
  await TestUtils.loadQuery("searchfox", "crash-set:0123456789abcdef");
  ok(doc().body.textContent.includes("There's no crash set 0123456789abcdef here"),
     "A set the store doesn't have says so");
});
