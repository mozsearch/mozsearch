"use strict";

// Tokens' timelines (the token-timeline: view; see
// tools/src/hyperblame/token_timeline.rs and static/js/token-timeline.js),
// from the "searchfox" tree's history of mozsearch itself: structopt, which
// its 2022 upgrade to clap v3 replaced, and clap, a line each, with markers at
// the weeks changing them the most, whose popups say what their commits
// changed, and link to the timelines of the other tokens they changed.

function doc() {
  return frame.contentDocument;
}

const UPGRADE = "0f2a4938884c2ce7e2637363c5577f095ef0d074";

add_task(async function test_TokenTimelineLines() {
  await TestUtils.loadQuery("searchfox", "token-timeline:structopt token-timeline:clap");
  await waitForCondition(() => doc().querySelectorAll(".token-timeline-line").length == 2,
                         "A line each");
  const legend = doc().querySelector(".token-timeline-legend");
  ok(legend.textContent.includes("with StructOpt"), "with structopt's case variant");
  // (The history's journals sum to their tokens' instances; see
  // check-token-counts, which the tree's setup runs.)
  ok(legend.textContent.match(/clap \(\+[\d,]+ −[\d,]+; [\d,]+ now\)/), "and their instances now");
  is(doc().querySelector(".token-timeline-axis-label").textContent, "Instances",
     "so the lines are of their instances");

  const marker = doc().querySelector('.token-timeline-marker[data-week="2022-W33"]');
  ok(marker, "The upgrade's week has a marker");
  marker.dispatchEvent(new frame.contentWindow.MouseEvent("click", { bubbles: true }));
  const popup = doc().querySelector(".token-timeline-popup");
  ok(!popup.hidden, "whose popup shows");
  const commit = popup.querySelector(`a[href="/searchfox/commit/${UPGRADE}"]`)?.closest("li");
  ok(commit, "with the upgrade's commit");
  ok(commit.textContent.includes("Upgrade to clap v3"), "and its message");
  ok(commit.querySelector(".token-timeline-commit-changes").textContent.includes("StructOpt"),
     "and its changes of the lines' tokens");

  const other = commit.querySelector(".token-timeline-other-link");
  ok(other, "and the other tokens it changed");
  ok(other.getAttribute("href").includes(encodeURIComponent(`token-timeline:${other.textContent}`)),
     "linking to their timelines");
  const add = commit.querySelector(".token-timeline-add").getAttribute("href");
  ok(add.includes("structopt") && add.includes("clap") && add.includes(other.textContent),
     "and to this timeline with them");
  ok(commit.querySelector(".token-timeline-all").textContent.match(/^all \d+ timelines$/),
     "and to all of theirs");
});

add_task(async function test_TokenTimelineScale() {
  await TestUtils.loadQuery("searchfox", "token-timeline:structopt token-timeline:clap");
  await waitForCondition(() => doc().querySelector(".token-timeline-line"), "The lines are there");
  const checkbox = doc().querySelector(".token-timeline-scale input");
  checkbox.click();
  ok(doc().querySelector(".token-timeline-axis-label").textContent.includes("within each line's range"),
     "Each line can fill the chart");
});

add_task(async function test_TokenTimelineBounds() {
  // (At most 16 tokens, case-insensitively distinct.)
  const tokens = ["structopt", "StructOpt"];
  for (let i = 0; i < 17; i++) {
    tokens.push(`token${i}`);
  }
  await TestUtils.loadQuery("searchfox", tokens.map(t => `token-timeline:${t}`).join(" "));
  await waitForCondition(() => doc().querySelector(".token-timeline-legend"), "The legend is there");
  is(doc().querySelectorAll(".token-timeline-legend li").length, 16, "16 tokens at most");
  ok(doc().querySelector(".token-timeline-dropped").textContent.includes("token15, token16"),
     "the others are left out");
});

add_task(async function test_TokenTimelineWithoutHistory() {
  await TestUtils.loadQuery("tests", "token-timeline:Foo");
  await waitForCondition(() => doc().querySelector(".token-timeline-empty"), "It says so");
  ok(doc().querySelector(".token-timeline-empty").textContent.includes("no token-centric history"),
     "A tree without a history has no timelines");
});

add_task(async function test_TokenTimelineApi() {
  const response = await fetch("/searchfox/api/v1/token-timeline?tokens=structopt,clap", { cache: "no-store" });
  is(response.status, 200, "The data API has timelines");
  const timeline = await response.json();
  is(timeline.series.map(s => s.token).join(","), "structopt,clap", "a line a token");
  ok(timeline.series.every(s => Number.isInteger(s.count)), "with their instances now");
  ok(timeline.markers.some(m => m.commits.some(c => c.rev == UPGRADE)), "with the markers' commits");
});

add_task(async function test_TokenHistoryLinksTimeline() {
  // (The note of a removed token links to its timeline with what replaced it.)
  await TestUtils.loadQuery("searchfox", "structopt");
  const link = doc().querySelector(".token-history-note .token-history-timeline");
  is(link?.getAttribute("href"),
     `/searchfox/query/default?q=${encodeURIComponent("token-timeline:structopt token-timeline:clap")}`,
     "The note links to the token's timeline with clap");
});
