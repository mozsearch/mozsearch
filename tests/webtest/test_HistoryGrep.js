"use strict";

// History greps (the text-history: and re-history: views; see
// tools/src/hyperblame/history_grep.rs), of the "searchfox" tree's history of
// mozsearch itself: where `structopt` came and went (its 2022 upgrade to clap
// v3 removed it), as `git log -S` finds, from its journals, and clap's
// version in tools/Cargo.toml over time, from that file's every revision.

function doc() {
  return frame.contentDocument;
}

const UPGRADE = "0f2a4938884c2ce7e2637363c5577f095ef0d074";

function rows() {
  return [...doc().querySelectorAll(".history-grep-table tbody tr")];
}

add_task(async function test_TextHistory() {
  await TestUtils.loadQuery("searchfox", "text-history:structopt");
  await waitForCondition(() => doc().querySelector(".history-grep"), "The view is there");
  ok(doc().querySelector(".history-grep-plan").textContent.includes("structopt"),
     "It says which journals it used");
  const upgrade = rows().filter(row => row.querySelector(`a[href="/searchfox/commit/${UPGRADE}"]`));
  ok(upgrade.length > 1, "The upgrade changed several files' matches");
  const cargo = upgrade.find(row => row.textContent.includes("tools/Cargo.toml"));
  ok(cargo, "including tools/Cargo.toml");
  ok(cargo.querySelector(".history-grep-count").textContent.includes("1 → 0"),
     "whose one match it removed");
  ok(cargo.querySelector(".history-grep-removed").textContent.includes("structopt"),
     "with the line it removed");
  ok(rows().some(row => row.querySelector(".history-grep-count").textContent.startsWith("0 →")),
     "and the revisions which added it are there too");
});

add_task(async function test_ReHistoryValues() {
  await TestUtils.loadQuery("searchfox",
                            `re-history:'^clap = .*version = "([^"]+)"' path:tools/Cargo.toml`);
  await waitForCondition(() => doc().querySelector(".history-grep"), "The view is there");
  ok(doc().querySelector(".history-grep-plan").textContent.includes("1 file"),
     "Its values come from the file's every revision");
  const upgrade = rows().find(row => row.querySelector(".history-grep-added")?.textContent.includes("4.0"));
  ok(upgrade, "A revision changed the version to 4.0");
  ok(upgrade.querySelector(".history-grep-removed").textContent.includes("3.0"),
     "from 3.0");

  const response = await fetch(
    TestUtils.getQueryPath("searchfox", `re-history:'^clap = .*version = "([^"]+)"' path:tools/Cargo.toml`),
    { headers: { Accept: "application/json" }, cache: "no-store" });
  const grep = (await response.json())?.HistoryGrep;
  ok(grep?.rows.some(row => row.added_values.some(v => v[0] == "4.0")),
     "The JSON has the values too");
});

add_task(async function test_HistoryGrepPages() {
  await TestUtils.loadQuery("searchfox", "text-history:structopt page:20");
  await waitForCondition(() => doc().querySelector(".history-grep"), "The view is there");
  ok(doc().querySelector(".history-grep-plan").textContent.includes("revisions 21–"),
     "A page starts where it says");
});

add_task(async function test_HistoryGrepTooExpensive() {
  await TestUtils.loadQuery("searchfox", "re-history:'\\w+'");
  await waitForCondition(() => doc().querySelector(".query-too-expensive"), "It's refused");
  ok(doc().querySelector(".query-too-expensive").textContent.includes("every revision"),
     "saying why");
  const response = await fetch(TestUtils.getQueryPath("searchfox", "re-history:'\\w+'"),
                               { headers: { Accept: "application/json" }, cache: "no-store" });
  is(response.status, 422, "A 422");
  is((await response.json())?.error, "query-too-expensive", "with its error code");
});

// A bot's history grep (by its user agent, from outside: with a forwarding
// header) is over its tier's limits, but not the server's, so it runs anyway,
// in the slow lane, rather than being refused; see tools/src/query_limits.rs.
add_task(async function test_HistoryGrepSlowLane() {
  const unique = Math.floor(Math.random() * 0xffff).toString(16);
  const response = await fetch(TestUtils.getQueryPath("searchfox", "text-history:structopt page:3"), {
    headers: {
      Accept: "application/json",
      "Fastly-Client-IP": `2001:db8:${unique}::7`,
      "User-Agent": "curl/8.5.0",
    },
    cache: "no-store",
  });
  is(response.status, 200, "A bot's history grep runs, rather than being refused");
  ok((await response.json())?.HistoryGrep, "with its results");
});

add_task(async function test_HistoryGrepWithoutHistory() {
  await TestUtils.loadQuery("tests", "text-history:Foo");
  await waitForCondition(() => doc().querySelector(".history-grep"), "The view is there");
  ok(doc().querySelector(".history-grep").textContent.includes("no history"),
     "A tree without a history has nothing to search");
});

add_task(async function test_ReTimelineValues() {
  await TestUtils.loadQuery("searchfox",
                            `re-timeline:'^clap = .*version = "([^"]+)"' path:tools/Cargo.toml`);
  await waitForCondition(() => doc().querySelector(".history-timeline-svg"), "The chart is there");
  ok(doc().querySelector(".history-timeline-table caption").textContent.includes("its value"),
     "Its line is the version's value");
  const values = [...doc().querySelectorAll(".history-timeline-table tbody tr .history-grep-count")]
        .map(cell => cell.textContent);
  ok(values.includes("3.0") || values.includes("3"), "It was 3");
  ok(values.includes("4.0") || values.includes("4"), "and then 4");
  is(doc().querySelectorAll(".history-timeline-marker").length, values.length,
     "A marker a change");
});

add_task(async function test_ReTimelineScope() {
  // (The pipeline builder's match of the commands, which gained arms as
  // commands were added, like an interpreter's switch's cases.)
  await TestUtils.loadQuery("searchfox", "re-timeline:'Command::(\\w+)\\(' scope:fab_command_from_opts");
  await waitForCondition(() => doc().querySelector(".history-timeline-table"), "The changes are there");
  ok(doc().querySelector(".history-grep-plan").textContent.includes("fab_command_from_opts"),
     "Only in the scope, found from its definition");
  const rows = [...doc().querySelectorAll(".history-timeline-table tbody tr")];
  const rename = rows.find(row => row.querySelector(".history-grep-added")?.textContent.includes("NormalizeUnstableData"));
  ok(rename, "A revision added NormalizeUnstableData");
  ok(rename.querySelector(".history-grep-removed")?.textContent.includes("ProductionFilter"),
     "and removed ProductionFilter, which it replaced");
  ok(rows.some(row => row.querySelector(".history-grep-added")?.textContent.includes("JQ")),
     "Another added JQ");
});

add_task(async function test_TextTimeline() {
  await TestUtils.loadQuery("searchfox", "text-timeline:structopt");
  await waitForCondition(() => doc().querySelector(".history-timeline-svg"), "The chart is there");
  ok(doc().querySelector(".history-timeline-table caption").textContent.includes("how many matches"),
     "Its line is the matches' count");
  const rows = [...doc().querySelectorAll(".history-timeline-table tbody tr")];
  const count = row => Number(row.querySelector(".history-grep-count").textContent);
  const upgrade = rows.findIndex(row => row.querySelector(`a[href="/searchfox/commit/${UPGRADE}"]`));
  ok(upgrade > 0, "The upgrade is a change");
  ok(count(rows[upgrade]) < count(rows[upgrade - 1]) / 2,
     "which removed most of its matches (some comments mention it)");
});

// A timeline checks all of its candidates at once, whatever its `page:`, even
// if they're more file versions than a page of its tier's checks (`unwrap`'s
// 1,049 in 603 revisions, in 2026, over an agent's 1,000): a page of a
// timeline would start its counts again.
add_task(async function test_TimelineIsOnePage() {
  const unique = Math.floor(Math.random() * 0xffff).toString(16);
  const response = await fetch(TestUtils.getQueryPath("searchfox", "text-timeline:unwrap page:3"), {
    headers: {
      Accept: "application/json",
      "Fastly-Client-IP": `2001:db8:${unique}::8`,
      "User-Agent": "searchfox-cli/0.3",
    },
    cache: "no-store",
  });
  const grep = (await response.json())?.HistoryGrep;
  ok(grep, "The agent's timeline runs");
  is(grep.page_start, 0, "from the first candidate");
  is(grep.page_end, grep.candidates, "to the last");
  is(grep.next_page, null, "with no next page");
  ok(grep.checked > 1000, `over a page's file versions (${grep.checked})`);

  await TestUtils.loadQuery("searchfox", "text-timeline:structopt page:3");
  await waitForCondition(() => doc().querySelector(".history-timeline-svg"), "The chart is there");
  ok(doc().querySelector(".history-grep-plan").textContent.includes("checked all of them"),
     "The plan says it checked all of the candidates");
});

// A timeline with more revisions than fit a marker each groups them by when
// they landed (`unwrap`'s 579 over 11 years, in 2026), a group's marker
// listing them and zooming into its time, where they have their own markers
// (or smaller groups).
add_task(async function test_TimelineGroups() {
  await TestUtils.loadQuery("searchfox", "text-timeline:unwrap");
  await waitForCondition(() => doc().querySelector(".history-timeline-svg"), "The chart is there");
  const groups = [...doc().querySelectorAll(".history-timeline-group")];
  ok(groups.length > 0, "Some markers are groups");
  ok(doc().querySelector(".history-timeline-note")?.textContent.includes("click to zoom in"),
     "which the note explains");
  const biggest = groups.sort((a, b) => b.dataset.count - a.dataset.count)[0];
  const count = Number(biggest.dataset.count);
  ok(biggest.querySelector("title").textContent.includes(`${count} revisions`),
     "A group's tooltip says how many revisions it has");
  biggest.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await waitForCondition(() => doc().querySelector(".history-timeline-zoom"), "Clicking it zooms in");
  const markers = doc().querySelectorAll(".history-timeline-marker").length;
  const grouped = [...doc().querySelectorAll(".history-timeline-group")]
        .reduce((n, group) => n + Number(group.dataset.count), 0);
  is(markers + grouped, count, "to its revisions");
  doc().querySelector(".history-timeline-zoom-out").click();
  await waitForCondition(() => !doc().querySelector(".history-timeline-zoom"), "Zooming out");
  is(doc().querySelectorAll(".history-timeline-group").length, groups.length,
     "goes back to the whole history's groups");
});

add_task(async function test_HistoryGrepIgnored() {
  // (A cut-off `until:` is text, which a history query says it ignored.)
  await TestUtils.loadQuery("searchfox", "text-history:structopt until");
  await waitForCondition(() => doc().querySelector(".history-grep"), "The view is there");
  ok(doc().querySelector(".history-grep-ignored")?.textContent.includes("until"),
     "It says what it ignored");
});
