"use strict";

// A Bugzilla comment's view (the `bug-comment:` query term's; see
// tools/src/bug_comment.rs): its blocks, each beside the excerpts of the code
// it links to, at the indexed revision, the linked lines mapped from the
// links' revisions and highlighted; and the DWIM rule sending Bugzilla
// comments' URLs there.  The comment is tests/bugzilla/'s fixture (see
// webtest-config.json's `bugzilla_url`), whose links are to the searchfox
// tree's tools/src/blame.rs at an old revision, whose lines have moved (or
// changed) since.

function doc() {
  return frame.contentDocument;
}

const blocks = () => [...doc().querySelectorAll(".bug-comment-block")];
const cards = block => [...block.querySelectorAll(".bug-comment-card")];
const heading = card => card.querySelector(".crash-code-heading a")?.textContent;
const note = card => card.querySelector(".crash-frame-where")?.textContent || "";
const highlighted = card => [...card.querySelectorAll(".crash-line")].map(line => line.textContent).join("\n");

add_task(async function test_BugComment() {
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567/1");
  ok(doc().querySelector(".crash-notice").textContent.startsWith(
    "Comment 1 on bug 1234567, Fix the frobnicator (Searchfox :: General, NEW), by someone@example.com on 2026-10-30"),
     "The view says which comment, of which bug");
  is(doc().querySelector(".bug-comment-link").getAttribute("href"),
     "https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c1", "and links to it");

  const [intro, moved, changed, now, crashed, code] = blocks();
  is(blocks().length, 6, "Its blocks are its paragraphs, its list's items and its code block");
  ok(intro.querySelector(".bug-comment-text code")?.textContent == "find_phab_rev" && cards(intro).length == 0,
     "A block without links to code has no excerpts");

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

  ok(crashed.querySelector(".bug-comment-crash a")?.href.includes("q=crash-report%3A5e4c0b1a-7e57-4c0d-8e5a-000000261030"),
     "A crash report's link is to its crash view");
  ok(code.querySelector("pre code") && cards(code).length == 0, "A code block is a block");

  // (Its description is comment 0; and a comment the bug doesn't have says so.)
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567");
  ok(doc().querySelector(".bug-comment-text")?.textContent.includes("The frobnicator doesn't frob."),
     "A bug's description is its comment 0");
  await TestUtils.loadQuery("searchfox", "bug-comment:1234567/9");
  ok(doc().body.textContent.includes("Bug 1234567 has no comment 9"), "A comment the bug doesn't have says so");
});

add_task(async function test_BugCommentDwim() {
  // (A search for a Bugzilla comment's URL goes to its view's query at once,
  // in a tree searching with queries.)
  await TestUtils.loadPath(`/trydelta/search?q=${encodeURIComponent("https://bugzilla.mozilla.org/show_bug.cgi?id=1234567#c1")}`);
  is(doc().location.pathname, "/trydelta/query/default", "A Bugzilla comment's URL goes to");
  is(new URLSearchParams(doc().location.search).get("q"), "bug-comment:1234567/1", "the query of its view");
  ok(doc().querySelector(".bug-comment-link"), "which shows it");
});
