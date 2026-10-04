"use strict";

// The interdiff of a reland from the landing it relands (see
// `format::format_interdiff`): dad860f9 (Bug 2016448) was backed out by
// e21bd3e0 and relanded as f8647c89, which made the directories with `cd` and
// `mkdir -p {}` rather than `mkdir -p '$INDEX_ROOT/file/{}'`.
const LANDING = "dad860f9";
const RELAND = "f8647c89";

add_task(async function test_InterdiffMarks() {
  await TestUtils.loadPath(`/searchfox/interdiff/${LANDING}/${RELAND}/scripts/mkdirs.sh`);

  const doc = frame.contentDocument;
  const texts = mark => [...doc.querySelectorAll(`#file .idiff-${mark}`)].map(span => span.textContent);
  ok(texts("new").includes("cd"), "The reland's new tokens are marked as new");
  ok(texts("same").length > 0, "The tokens the landing added too are marked as the same");

  const onlyA = [...doc.querySelectorAll(".interdiff-only-a")];
  ok(onlyA.length > 0, "The landing's lines with tokens the reland doesn't have are shown");
  ok(onlyA.every(row => row.querySelector(".idiff-only-a")),
     "with the tokens the reland doesn't have marked");

  ok(doc.querySelector(".interdiff-legend").textContent.includes("new in B"),
     "The summary explains the marks");
});

add_task(async function test_InterdiffFiles() {
  await TestUtils.loadPath(`/searchfox/interdiff/${LANDING}/${RELAND}`);

  const links = [...frame.contentDocument.querySelectorAll(".interdiff-file-list a")];
  is(links.length, 1, "The landing and the reland changed one file");
  ok(links[0].href.endsWith("/scripts/mkdirs.sh"), "The file links to its interdiff");
});

add_task(async function test_ExploreRelandLink() {
  await TestUtils.loadPath("/searchfox/explore/bug/2016448");

  const link = frame.contentDocument.querySelector(".explore-interdiff");
  ok(link, "The backout links to the interdiff of its reland");
  ok(link.href.includes(`/interdiff/${LANDING}`) && link.href.includes(`/${RELAND}`),
     "The interdiff is of the reland from the landing");
});
