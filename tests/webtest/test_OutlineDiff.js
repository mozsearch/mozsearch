"use strict";

// The revision of the page and the parent of the newest commit changing its
// file, which differs from it.
async function revisions() {
  const win = frame.contentWindow;
  const head = frame.contentDocument.querySelector("#rev-id a").getAttribute("href").split("/").pop();
  const timeline = win.FileTimeline;
  const newest = timeline.commits[timeline.commits.length - 1];
  const headers = await (await win.fetch(`${timeline.info.dataUrl}/commits.json`)).json();
  return { head, newest, parent: headers[newest.index].parent };
}

// How a file's outline changed between two revisions, as JSON (see
// `hyperblame::outline_diff`).
add_task(async function test_OutlineJSON() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const win = frame.contentWindow;
  const { head, newest, parent } = await revisions();

  const diff = await (await win.fetch(`/searchfox/rev-outline/${parent}/${head}/tools/src/format.rs`)).json();
  is(diff.to.rev, head, "The outline diff is to the revision");
  ok(diff.from.rev.startsWith(parent), "from the baseline");
  ok(diff.complete, "with all of the records between them");
  ok(diff.commits.some(commit => commit.rev == newest.rev && commit.subject),
     "which have the file's newest commit");
  ok(diff.entries.length > 10, "It has the file's definitions");
  ok(diff.entries.some(entry => entry.status != "unchanged" && (entry.changes || []).some(([i]) => diff.commits[i].rev == newest.rev)),
     "and which of them the newest commit changed");
  ok(diff.entries.every(entry => entry.depth >= 0 && entry.name && entry.pretty.endsWith(entry.name)),
     "with their nesting");

  // (A baseline by date: the newest revision before it.)
  const day = new Date(newest.time * 1000).toISOString().slice(0, 10);
  const since = await (await win.fetch(`/searchfox/rev-outline/since:${day}/${head}/tools/src/format.rs`)).json();
  ok(since.from.date < day, "A baseline by date is the newest revision before it");
  ok(since.commits.some(commit => commit.rev == newest.rev), "so the range has the commits since");
});
