"use strict";

// Search results' lines in specs' sources (and in the files Bikeshed specs
// include) go to their blocks in the rendered views (see the tools'
// `format::rendered_line_template`), and code's to the source listings; the
// files' headings go to the source listings.  A rendered view's link to a
// line shows its block, the panel's "Source listing" going to the line.  The
// tests tree's "poke" is the example spec's (a Wattsi source, split into
// pages), the Bikeshed spec's (whose example-gizmos.md include mentions it),
// and code's.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_SpecResultLinks() {
  await TestUtils.loadQuery("tests", "poke");
  const links = [...doc().querySelectorAll("a.query-line-number")];
  const href = link => link.getAttribute("href");
  const some = pattern => links.find(link => pattern.test(href(link)));
  ok(some(/^\/tests\/rendered\/specs\/source\/line\/\d+$/),
     "A paged spec's source's lines go to its rendered view's page with them");
  ok(some(/^\/tests\/rendered\/specs\/example\.bs#\d+$/), "another spec's, to its rendered view's line");
  ok(some(/^\/tests\/rendered\/specs\/example-gizmos\.md#\d+$/),
     "and a file a Bikeshed spec includes, to its part's rendered view");
  ok(some(/^\/tests\/source\/spec_citations\.cpp#\d+$/), "Code's go to its source listing");
  ok(links.filter(link => link.classList.contains("query-line-rendered"))
       .every(link => link.title.includes("in the rendered view")),
     "which the rendered ones' titles say");
  const details = [...doc().querySelectorAll("details")]
    .find(each => each.querySelector('a.query-line-number[href^="/tests/rendered/specs/source/line/"]'));
  ok([...details.querySelectorAll("summary a")].some(a => a.getAttribute("href") == "/tests/source/specs/source"),
     "The spec source's heading goes to its source listing");

  // (Following a line's link shows its block, and the panel's source listing
  // link goes to the line.)
  const link = some(/^\/tests\/rendered\/specs\/source\/line\/\d+$/);
  const line = href(link).split("/").pop();
  await TestUtils.loadPath(href(link));
  is(frame.contentWindow.location.hash, `#${line}`, "The line's page has it as its hash");
  ok(doc().querySelector(".rd-hash-target")?.textContent.toLowerCase().includes("poke"),
     "showing its block, which has the result's line");
  ok(doc().querySelector("#panel-source-listing").href.endsWith(`/tests/source/specs/source#${line}`),
     "and the panel's source listing goes to the line");
});
