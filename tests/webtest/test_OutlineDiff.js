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

// The fancy bar's outline lists the definitions which changed since the
// baseline, which the file timeline's "since" picks, and goes to them.
add_task(async function test_OutlineSection() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const doc = frame.contentDocument;
  const section = doc.querySelector("#panel-outline");
  ok(section, "The fancy bar has the file's outline");
  const summary = section.querySelector(".outline-summary");
  await waitForCondition(() => !summary.textContent.startsWith("Loading"), "which loads");
  const timeline = frame.contentWindow.FileTimeline;
  is(section.querySelector(".outline-baseline button").textContent,
     timeline.day(timeline.revTime - 28 * 24 * 60 * 60),
     "with the changes since 4 weeks before the revision by default");
  let entries = [...section.querySelectorAll(".outline-entry")];
  ok(entries.length > 10, "It has the file's definitions");
  const shown = () => entries.filter(entry => !entry.hidden);
  const all = section.querySelector(".outline-all input");
  TestUtils.click(all);
  is(shown().length, entries.length, "\"All\" shows all of them");
  TestUtils.click(all);
  const depth = entry => Number(entry?.style.getPropertyValue("--outline-depth") ?? -1);
  ok(shown().every(entry => !entry.classList.contains("outline-unchanged") ||
                            depth(entry.nextElementSibling) > depth(entry)),
     "and otherwise the ones which changed, and the ones they're in");

  // (Since the file's newest commit, on the timeline.)
  TestUtils.click(doc.querySelector("#revision button.file-timeline"));
  const popup = doc.querySelector(".file-timeline-popup");
  await waitForCondition(() => popup.querySelector(".file-timeline-commits li"), "The timeline lists the commits");
  const newest = popup.querySelector(".file-timeline-commits li");
  TestUtils.click(newest.querySelector(".file-timeline-extras button"));
  TestUtils.click(popup.querySelector(".lens-popup-close"));
  const date = newest.querySelector("time").textContent;
  is(section.querySelector(".outline-baseline button").textContent, date,
     "The timeline's \"since\" is the outline's baseline");
  await waitForCondition(() => !summary.textContent.startsWith("Loading"), "which loads again");
  entries = [...section.querySelectorAll(".outline-entry")];
  const changed = shown().filter(entry => entry.querySelector(".outline-badge"));
  ok(changed.length > 0, "with the definitions the commit changed");
  ok(changed.some(entry => entry.querySelector(".outline-spark rect")), "and their sparklines");

  const link = shown().map(entry => entry.querySelector("a.outline-name[href^='#']")).find(Boolean);
  ok(link, "A definition links to its line");
  TestUtils.click(link);
  await waitForCondition(() => frame.contentWindow.location.hash == link.getAttribute("href"),
                         "Clicking it goes to its line");
  ok(doc.getElementById(`line-${link.getAttribute("href").slice(1)}`).classList.contains("highlighted"),
     "which it selects");
});

// The page of an outline diff, from the timeline's commits: the file's
// definitions before and after the commit side by side, with their changes.
add_task(async function test_OutlineDiffPage() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  let doc = frame.contentDocument;
  const { newest, parent } = await revisions();
  TestUtils.click(doc.querySelector("#revision button.file-timeline"));
  const popup = doc.querySelector(".file-timeline-popup");
  await waitForCondition(() => popup.querySelector(".file-timeline-commits li"), "The timeline lists the commits");
  const link = [...popup.querySelectorAll(".file-timeline-commits li")[0].querySelectorAll(".file-timeline-extras a")]
    .find(a => a.textContent == "outline");
  is(link?.getAttribute("href"), `/searchfox/outline-diff/${parent}/${newest.rev}/tools/src/format.rs`,
     "A commit links to how it changed the file's outline");
  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  doc = frame.contentDocument;
  ok(doc.querySelector(".outline-diff h2").textContent.includes("tools/src/format.rs"), "The page is the file's outline diff");
  const rows = [...doc.querySelectorAll(".outline-diff-row")];
  ok(rows.length > 10, "with its definitions");
  const visible = row => frame.contentWindow.getComputedStyle(row).display != "none";
  const changed = rows.filter(row => visible(row) && row.querySelector(".outline-badge"));
  ok(changed.length > 0, "and the ones the commit changed");
  ok(changed.some(row => row.querySelector(".outline-diff-commit")?.getAttribute("href") == "#commit-1"),
     "which link to the commit");
  is(doc.querySelector("#commit-1 a").getAttribute("href"), `/searchfox/commit/${newest.rev}`,
     "in the list of the commits");
  const to = changed.map(row => row.querySelector(".outline-diff-to a")).find(Boolean);
  ok(to?.getAttribute("href").startsWith(`/searchfox/rev/${newest.rev}/tools/src/format.rs#`),
     "Definitions link to their lines");
  const unchanged = rows.find(row => row.matches(".outline-unchanged:not(.outline-context)"));
  ok(unchanged && !visible(unchanged), "The unchanged definitions are hidden");
  TestUtils.click(doc.querySelector("#outline-diff-all"));
  ok(visible(unchanged), "unless asked for");
});

// The outline section and explore pages link to outline diffs too.
add_task(async function test_OutlineDiffLinks() {
  await TestUtils.loadPath("/searchfox/source/tools/src/format.rs");
  const section = frame.contentDocument.querySelector("#panel-outline");
  await waitForCondition(() => section.querySelector(".outline-compare"), "The outline loads");
  ok(/^\/searchfox\/outline-diff\/since%3A\d{4}-\d{2}-\d{2}\/[0-9a-f]+\/tools\/src\/format\.rs$/
       .test(section.querySelector(".outline-compare").getAttribute("href")),
     "The outline section links to the outline diff since its baseline");

  await TestUtils.loadPath("/searchfox/explore/bug/1975601");
  const file = frame.contentDocument.querySelector(".explore-file .explore-outline");
  ok(/^\/searchfox\/outline-diff\/[0-9a-f]{40}\/[0-9a-f]{40}\//.test(file?.getAttribute("href")),
     "An explore page's files link to their outline diffs across its commits");
});
