"use strict";

// A query's `path:` keeps what the rest of it finds in the paths it names
// (see query_core.toml's `pathre` term): the files whose paths also match
// what it searches for, the text in them, and its semantic results' in them;
// and `-path:` leaves its semantic results' out too.  (The fixtures in
// tests/tests/files/inpath.)

function doc() {
  return frame.contentDocument;
}

// (A file's result, among the files whose names match; not a file's results'
// heading, which links to it too.)
const fileResult = path => doc().querySelector(`.query-file-names li[data-path="${path}"]`);
const fileLines = path => doc().querySelector(`details[data-path="${path}"]`);

const INSIDE = "inpath/inside/PathProbe.cpp";
const INSIDE_USER = "inpath/inside/inside_user.cpp";
const OUTSIDE = "inpath/outside/PathProbeOutside.cpp";

add_task(async function test_QueryPath() {
  await TestUtils.loadQuery("tests", "pathprobe");
  ok(fileResult(INSIDE) && fileResult(OUTSIDE), "A query finds the files with its text in their names");
  ok(!fileResult(INSIDE_USER), "not others");
  ok(fileLines(INSIDE_USER) && fileLines(OUTSIDE), "and the symbol's uses everywhere");

  await TestUtils.loadQuery("tests", "path:inpath/inside pathprobe");
  ok(fileResult(INSIDE), "With a path, it finds the files with its text in their names in the path");
  ok(!fileResult(OUTSIDE), "not those elsewhere");
  ok(!fileResult(INSIDE_USER), "nor the path's others");
  ok(fileLines(INSIDE) && fileLines(INSIDE_USER), "and the symbol's definition and uses in the path");
  ok(!fileLines(OUTSIDE), "not those elsewhere");
  const paths = [...doc().querySelectorAll("details[data-path]")].map(details => details.dataset.path);
  ok(paths.length && paths.every(path => path.startsWith("inpath/inside/")),
     `nor anything else outside the path (${paths.join(", ")})`);

  await TestUtils.loadQuery("tests", "pathprobe -path:inpath/outside");
  ok(fileLines(INSIDE_USER), "Leaving out a path keeps the symbol's uses elsewhere");
  ok(!fileLines(OUTSIDE), "but not those in the path");
  ok(!fileResult(OUTSIDE), "nor the files there");
});
