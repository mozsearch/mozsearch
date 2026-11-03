"use strict";

// A query's file search ignores case, as router.py's `/search/` does, but with
// `case:sensitive` (see tools/src/cmd_pipeline/cmd_search_files.rs and
// query_core.toml's `case` term): "cargo.toml" finds tools/Cargo.toml.

function doc() {
  return frame.contentDocument;
}

// (A file's result links to its source listing, without a line.)
const fileResult = () => doc().querySelector('a[href="/searchfox/source/tools/Cargo.toml"]');

add_task(async function test_QueryFilesCase() {
  await TestUtils.loadQuery("searchfox", "cargo.toml");
  ok(fileResult(), "A query's file search ignores case");
  await TestUtils.loadQuery("searchfox", "Cargo.toml");
  ok(fileResult(), "so it finds the file by its own case too");
  await TestUtils.loadQuery("searchfox", "cargo.toml case:sensitive");
  ok(!fileResult(), "but with case:sensitive, only by its own case");
});
