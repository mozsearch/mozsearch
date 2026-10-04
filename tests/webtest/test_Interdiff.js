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

  const doc = frame.contentDocument;
  const files = [...doc.querySelectorAll(".explore-file")];
  is(files.length, 1, "The landing and the reland changed one file");
  ok(files[0].querySelector(".explore-file-header a").href.endsWith("/scripts/mkdirs.sh"),
     "The file links to its interdiff");
  ok(!files[0].classList.contains("interdiff-same"),
     "The file isn't faded, since the patches differ in it");
  is(doc.querySelectorAll(".explore-commit").length, 2,
     "The sides' commits are listed, for the sparklines");

  ok(files[0].querySelector(".interdiff-excerpt"),
     "The summary has excerpts of where the patches differ");
  const newTokens = [...files[0].querySelectorAll(".interdiff-excerpt .idiff-new")];
  ok(newTokens.some(span => span.textContent == "cd"), "with the reland's new tokens marked");
});

add_task(async function test_InterdiffFilesSame() {
  await TestUtils.loadPath(`/searchfox/interdiff/${LANDING}/${LANDING}`);

  const doc = frame.contentDocument;
  const files = [...doc.querySelectorAll(".explore-file")];
  ok(files.length > 0 && files.every(file => file.classList.contains("interdiff-same")),
     "A patch's interdiff with itself fades all of its files");
  is(doc.querySelectorAll(".interdiff-excerpt").length, 0, "and has no excerpts");
});

add_task(async function test_ExploreRelandLink() {
  await TestUtils.loadPath("/searchfox/explore/bug/2016448");

  const link = frame.contentDocument.querySelector(".explore-interdiff");
  ok(link, "The backout links to the interdiff of its reland");
  ok(link.href.includes(`/interdiff/${LANDING}`) && link.href.includes(`/${RELAND}`),
     "The interdiff is of the reland from the landing");
});
