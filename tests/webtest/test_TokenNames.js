"use strict";

// The token-name index (see tools/src/file_format/token_names.rs, built at
// index time by build-token-names) and the token-names: view: the tokens of
// the "searchfox" tree's history's journals matching a pattern.

function doc() {
  return frame.contentDocument;
}

function listed() {
  return [...doc().querySelectorAll(".token-names-list li code")].map(code => code.textContent);
}

add_task(async function test_TokenNamesContains() {
  await TestUtils.loadQuery("searchfox", "token-names:structopt");
  is(doc().querySelector(".token-names-view h2").textContent, "Tokens containing structopt",
     "The view says what it lists");
  is(listed().join(","), "StructOpt,structopt", "the tokens containing it, case-insensitively");
  const links = doc().querySelector(".token-names-list li").querySelectorAll("a");
  ok(links[0].getAttribute("href").includes("token-history:StructOpt%20case:sensitive"),
     "each with a link to its history");
  ok(links[1].getAttribute("href").includes("token-timeline:StructOpt%20case:sensitive"),
     "and to its timeline");
});

add_task(async function test_TokenNamesPrefixSuffix() {
  await TestUtils.loadQuery("searchfox", "token-names:clap*");
  ok(listed().length > 0 && listed().every(name => name.toLowerCase().startsWith("clap")),
     "A prefix's tokens start with it");
  await TestUtils.loadQuery("searchfox", "token-names:*Opt");
  ok(listed().length > 1 && listed().every(name => name.toLowerCase().endsWith("opt")),
     "A suffix's tokens end with it");
  ok(listed().includes("StructOpt") && listed().includes("adopt"), "case-insensitively");
});

add_task(async function test_TokenNamesWithoutHistory() {
  await TestUtils.loadQuery("tests", "token-names:Foo");
  ok(doc().querySelector(".token-names-view").textContent.includes("no token-centric history"),
     "A tree without a history has no token names");
});

add_task(async function test_TokenNamesApi() {
  const response = await fetch("/searchfox/api/v1/token-names?q=struct&match=prefix&limit=5", { cache: "no-store" });
  is(response.status, 200, "The data API has the token names");
  const matches = await response.json();
  is(matches.names.length, 5, "up to the limit");
  ok(matches.more, "saying there are more");
  ok(matches.names.every(name => name.toLowerCase().startsWith("struct")), "matching");
});
