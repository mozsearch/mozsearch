"use strict";

// /query/ shows forward declarations (after the other semantic results, and
// collapsed) and type aliases (ex: typedefs), like router.py's "Aliases".

function kindGroup(doc, name) {
  return [...doc.querySelectorAll(".query-result details.facet-group")]
    .find(group => group.querySelector(":scope > summary > h2 .query-toggle")?.textContent == name);
}

add_task(async function test_QueryForwardDeclarations() {
  await TestUtils.loadQuery("tests", "CallerOne");

  const doc = frame.contentDocument;
  const forwards = kindGroup(doc, "Forward Declarations");
  ok(forwards, "Forward declarations are shown");
  ok(!forwards.open, "collapsed");
  ok(forwards.querySelector("#line-3 mark.query-hit"), "with `class CallerOne;`");
  const text = kindGroup(doc, "Textual Occurrences");
  const groups = [...doc.querySelectorAll(".query-result details.facet-group:has(> summary > h2)")];
  is(groups[groups.indexOf(forwards) + 1], text,
     "after the other semantic results, before the textual occurrences");

  const textFile = text && [...text.querySelectorAll("details")]
    .find(file => file.querySelector("h3.path")?.textContent.trim() == "lots_of_calls.cpp");
  ok(!textFile?.querySelector("#line-3"), "and not as textual occurrences");
});

add_task(async function test_QueryAliases() {
  await TestUtils.loadQuery("tests", "OtherR");

  const doc = frame.contentDocument;
  const aliases = kindGroup(doc, "Aliases");
  ok(aliases, "Type aliases are shown");
  ok(aliases.querySelector("mark.query-hit")?.textContent == "OtherR", "with the typedef");
});
