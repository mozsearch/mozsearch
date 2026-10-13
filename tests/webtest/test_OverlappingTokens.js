"use strict";

// A spec-html file which the tokenizer misreads into overlapping tokens (an
// ideographic space after a tag name; see tests/spechist/revs/5 and
// page_blame's test_overlapping_tokens) has its page, with its blame.
add_task(async function test_OverlappingTokens() {
  await TestUtils.loadPath("/spechist/source/hist/notes.html");
  ok(frame.contentDocument.querySelector("#file"), "The file's page is there");
  ok(frame.contentWindow.BLAME_INFO?.dataUrl, "with its token-centric blame");
  ok(frame.contentDocument.querySelector("#line-3 .blame-strip")?.dataset.hyperblame,
     "including the line with the overlapping tokens");
});
