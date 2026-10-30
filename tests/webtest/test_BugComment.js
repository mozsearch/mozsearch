"use strict";

// A Bugzilla comment's view (the `bug-comment:` query term's; see
// tools/src/bug_comment.rs): the comment as Bugzilla shows it (its bug, its
// header, its text as Markdown, or as plain text), its blocks each beside the
// cards of what they link to: the excerpts of the code they link to, at the
// indexed revision, the linked lines mapped from the links' revisions and
// highlighted, crash reports, and the frames they list beside their code;
// what the view couldn't get, which it says; and the DWIM rule sending
// Bugzilla comments' URLs there.  The comments are tests/bugzilla/'s
// fixtures (see webtest-config.json's `bugzilla_url`), whose links are to the
// searchfox tree's tools/src/blame.rs at an old revision, whose lines have
// moved (or changed) since.

function doc() {
  return frame.contentDocument;
}

const blocks = () => [...doc().querySelectorAll(".bug-comment-block")];
const cards = block => [...block.querySelectorAll(".bug-comment-cards > .bug-comment-card")];
const heading = card => card.querySelector(".crash-code-heading a")?.textContent;
const note = card => card.querySelector(".crash-frame-where")?.textContent || "";
const highlighted = card => [...card.querySelectorAll(".crash-line")].map(line => line.textContent).join("\n");
const text = selector => doc().querySelector(selector)?.textContent.trim();

add_task(async function test_BugComment() {
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567/1");
  is(text(".bug-comment-bug-id"), "Bug 1234567", "The view says which bug");
  is(text(".bug-comment-bug-summary"), "Fix the frobnicator", "with its summary");
  is(text(".bug-comment-bug-about"), "Searchfox :: General · NEW", "where it is, and its status");
  is(text(".bmo-author"), "Sam Someone [:sam]", "The comment's author is named as Bugzilla does");
  ok(doc().querySelector(".bmo-author").href.endsWith("/user_profile?user_id=12"), "linked to their profile");
  is(text(".bmo-role"), "Assignee", "with their role in the bug");
  is(text(".bug-comment-link"), "Comment 1", "It says which comment");
  is(doc().querySelector(".bug-comment-link").getAttribute("href"),
     "https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c1", "and links to it");
  ok(text(".bmo-change-time").endsWith(" ago"), "and how long ago it was written");
  is(doc().querySelector(".bmo-change-time").title, "2026-10-30 12:00 UTC", "when, in its title");
  ok(!doc().querySelector(".query-problem"), "It got all it needed");

  const [intro, moved, changed, now, crashed, frames] = blocks();
  is(blocks().length, 6, "Its blocks are its paragraphs, its list's items and its code block");
  ok(blocks().every(block => block.firstElementChild.matches(".bug-comment-text.markdown-body")),
     "each its text, as Markdown, first, beside its cards");
  ok(intro.querySelector(".bug-comment-text code")?.textContent == "find_phab_rev" && cards(intro).length == 0,
     "A block without links to code has no excerpts");
  ok(moved.matches(".bug-comment-item") && !now.matches(".bug-comment-item"),
     "A list's items, but its last, are followed by its next ones");

  const [movedCard] = cards(moved);
  ok(highlighted(movedCard).includes("line.splitn(4, ':')"), "A link's line is highlighted where it is now");
  ok(/^tools\/src\/blame\.rs#\d+$/.test(heading(movedCard)) && heading(movedCard) != "tools/src/blame.rs#147",
     "having moved since");
  is(note(movedCard), "Lines 147 at its revision.", "which it says");
  ok(movedCard.querySelector(".bug-comment-as-linked").href.includes("/rev/50df75fc5253890347bfac70037bc7ad42316862/"),
     "with a link to the code as linked");

  const [changedCard] = cards(changed);
  is(note(changedCard), "Its lines have changed since.", "Lines which changed since say so");
  ok(/^tools\/src\/blame\.rs, near line \d+$/.test(heading(changedCard)), "and where they would be now");
  is(changedCard.querySelectorAll(".crash-line").length, 0, "and aren't highlighted");
  ok(changedCard.querySelectorAll("[data-line-number]").length > 0, "but where they would be is shown");

  const [current, other] = cards(now);
  is(note(current), "Not a permalink: its lines now.", "A link which isn't a permalink is to its lines now");
  ok(highlighted(current).includes("fn find_phab_rev"), "which are highlighted");
  is(other?.querySelector(".crash-code-missing")?.textContent, "Not a file in this tree.",
     "A nested list's link is its item's, and a file the tree doesn't have says so");
  is(note(other), "", "(only that)");

  const [crashCard] = cards(crashed);
  ok(crashCard.matches(".bug-comment-crash") && crashCard.textContent.includes("Crash report 5e4c0b1a"),
     "A crash report's link is its report's card");
  ok(crashCard.textContent.includes("Searchfox 1.0a1"), "saying what crashed");
  ok(crashCard.querySelector(".bug-comment-crash-view").href.includes("q=crash-report%3A5e4c0b1a-7e57-4c0d-8e5a-000000261030"),
     "linked to its crash view");
  is(crashCard.querySelectorAll(".crash-frame").length, 0, "without its frames, which the comment lists");

  ok(frames.querySelector("pre code"), "A code block is a block");
  const [framesCard] = cards(frames);
  ok(framesCard?.matches(".bug-comment-frames") && note(framesCard).startsWith("Its frames, from the crash report"),
     "and one listing a crash's frames is beside the report's frames");
  const reportFrames = [...framesCard.querySelectorAll(".crash-frame")];
  is(reportFrames.length, 2, "as far as it lists them (frame 0), with their inlined calls");
  ok(reportFrames[1].querySelector(".crash-frame-name").textContent.startsWith("tools::blame::find_phab_rev"),
     "each beside its code");
  ok(reportFrames[1].querySelector(".crash-frame-where").textContent.includes("the build's revision"),
     "at the build's revision");

  // (Its description is comment 0, by its reporter.)
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567");
  ok(doc().querySelector(".bug-comment-text")?.textContent.includes("The frobnicator doesn't frob."),
     "A bug's description is its comment 0");
  is(text(".bug-comment-link"), "Description", "which Bugzilla calls its description");
  is(text(".bmo-role"), "Reporter", "by its reporter");
});

add_task(async function test_BugCommentEstimated() {
  // (Comment 2's crash report, which crash-stats doesn't have, is said, and
  // its listed frames' lines are at the tree's head when it was submitted, by
  // its ID: 2099-12-31's, the tree's head.)
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567/2");
  const [crashed, , listed] = blocks();
  ok(cards(crashed)[0]?.querySelector(".view-problem")?.textContent.includes(
       "crash-stats doesn't have the crash report 00000000-0000-4000-8000-000000991231"),
     "A crash report crash-stats doesn't have says so");
  ok(!doc().querySelector(".query-problem"), "(on its card)");

  const [framesCard] = cards(listed);
  ok(note(framesCard).startsWith("The crash report isn't available, so these are its frames' lines at searchfox's head"),
     "Its listed frames are at the tree's head then");
  ok(framesCard.querySelector(".crash-frame-where a")?.getAttribute("href").startsWith("/searchfox/rev/"),
     "which it links to");
  const frames = [...framesCard.querySelectorAll(".crash-frame")];
  is(frames.length, 3, "each listed frame");
  ok(frames[0].querySelector(".crash-frame-where").textContent.includes("the tree's revision then"),
     "at the tree's revision then");
  ok(frames[0].querySelectorAll(".crash-rows [data-line-number]").length > 0, "beside its code");
  ok(frames[2].querySelector(".crash-code-missing"), "and a frame without a file without code");
});

add_task(async function test_BugCommentByName() {
  // (In a tree without a history, the listed frames are found by their
  // functions' names: rollup/engine.cpp's Engine::Run calls Storage::Put at
  // lines 8 and 9; see test_CrashStack.)
  await TestUtils.loadQuery("tests", "bug-comment:1234567/3");
  const [, , listed] = blocks();
  const [framesCard] = cards(listed);
  ok(note(framesCard).startsWith("These are its frames' functions in the tree now, found by their names"),
     "A tree without the revision then finds the listed frames' functions by their names");
  const run = framesCard.querySelectorAll(".crash-frame")[1];
  is(run?.querySelector(".crash-frame-name")?.textContent, "rollup::Engine::Run", "as a signature's");
  is([...run.querySelectorAll(".crash-line [data-line-number]")].map(line => line.dataset.lineNumber).join(","),
     "8,9", "with their calls of the frame above highlighted");
});

add_task(async function test_BugCommentPlain() {
  // (Comment 4 is one Bugzilla shows as plain text; see
  // tests/bugzilla/show_bug.cgi.html.)
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567/4");
  const [quoting, linking] = blocks();
  ok(!quoting.querySelector(".bug-comment-text").matches(".markdown-body"), "A plain text comment isn't Markdown");
  ok(quoting.querySelector(".quote")?.textContent.startsWith("> Looking at how `find_phab_rev`"),
     "Its quoted lines are quotes");
  is(quoting.querySelector("a.bmo-mention")?.getAttribute("href"),
     "https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c1", "Its mentions of comments are linked");
  ok(linking.querySelector(".bug-comment-text").textContent.includes("*this* isn't emphasis"),
     "and it isn't Markdown");
  is(linking.querySelector("a.bmo-mention")?.getAttribute("href"),
     "https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c0", "as are bugs' comments");
  is(note(cards(linking)[0]), "Not a permalink: its lines now.", "and its code links are excerpts");
  is(text(".bmo-role"), "Reporter", "(by the bug's reporter)");
});

add_task(async function test_BugCommentProblems() {
  // (A view which fails says why, as a query's page, not cached.)
  const query = "bug-comment:1234567/9";
  await TestUtils.loadQuery("searchfox", query);
  ok(text(".query-problem")?.includes("Bug 1234567 has no comment 9"), "A comment the bug doesn't have says so");
  const response = await frame.contentWindow.fetch(TestUtils.getQueryPath("searchfox", query), { cache: "no-store" });
  is(response.status, 400, "as a 400");
  await TestUtils.loadQuery("searchfox", "crash-report:00000000-0000-4000-8000-000000991231");
  ok(text(".query-problem")?.includes("crash-stats doesn't have the crash report"),
     "and a crash report crash-stats doesn't have, the crash view");
});

add_task(async function test_BugCommentDwim() {
  // (A search for a Bugzilla comment's URL goes to its view's query at once,
  // in a tree searching with queries.)
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent("https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c1")}`);
  is(doc().location.pathname, "/trydelta/query/default", "A Bugzilla comment's URL goes to");
  is(new URLSearchParams(doc().location.search).get("q"), "bug-comment:1234567/1", "the query of its view");
  ok(doc().querySelector(".bug-comment-link"), "which shows it");
});
