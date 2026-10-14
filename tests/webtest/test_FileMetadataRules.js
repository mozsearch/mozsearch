"use strict";

// Files' metadata links to the rules giving it (see tools/src/file_metadata):
// the moz.build rule giving a file its Bugzilla component, review-context.toml's
// rules for the review guidance applying to it, and its test manifests'
// sections and conditions.

function panelItem(label) {
  return [...frame.contentDocument.querySelectorAll("#panel .item")]
    .find(item => item.textContent.startsWith(label));
}

add_task(async function test_BugComponentRule() {
  await TestUtils.loadPath("/tests/source/mozbuild/test.cpp");
  const fileBug = panelItem("File a bug in Core :: Build Fixtures");
  ok(fileBug, "The file's component is from bugzilla-components.json");
  const rule = fileBug.parentNode.querySelector(".item-extra");
  is(rule?.textContent, "moz.build", "Beside it is a link to its moz.build rule");
  is(rule?.getAttribute("href"), "/tests/source/mozbuild/moz.build#51",
     "at the line giving the component");

  // (A moz.build rule which doesn't agree with bugzilla-components.json
  // isn't linked.)
  await TestUtils.loadPath("/tests/source/mozbuild/header.h");
  const other = panelItem("File a bug in Core :: Simple");
  ok(other, "The file's component is bugzilla-components.json's");
  ok(!other.parentNode.querySelector(".item-extra"),
     "and its moz.build rule, which says otherwise, isn't linked");
});

add_task(async function test_ReviewSkills() {
  await TestUtils.loadPath("/tests/source/mozbuild/test.cpp");
  const skill = panelItem("Review skill: Build fixtures");
  ok(skill, "A review-context.toml rule applies to the file");
  is(skill.getAttribute("href"), "/tests/rendered/docs/md_test/index.md",
     "It goes to its document's rendered view");
  is(skill.querySelector("button.copy")?.dataset.copy, "docs/md_test/index.md",
     "Its copy button copies the document's path");
  is(skill.parentNode.querySelector(".item-extra")?.getAttribute("href"),
     "/tests/source/review-context.toml#6", "Beside it is a link to the rule");

  // A rule which also needs other files' changes says so, with an item for
  // each of its loads.
  await TestUtils.loadPath("/tests/source/tests/test_otherthing.html");
  const items = [...frame.contentDocument.querySelectorAll("#panel .item")]
    .filter(item => item.textContent.startsWith("Review skill: Tests and their manifests"));
  is(JSON.stringify(items.map(item => [item.firstChild.textContent, item.getAttribute("href")])),
     JSON.stringify([
       ["Review skill: Tests and their manifests (tests/head.js), with other changes",
        "/tests/source/tests/head.js"],
       ["Review skill: Tests and their manifests (mozilla/other-repo docs/guide.md), with other changes",
        "https://github.com/mozilla/other-repo/blob/main/docs/guide.md"],
       ["Review skill: Tests and their manifests (D12345), with other changes",
        "https://phabricator.services.mozilla.com/D12345"],
     ]),
     "A partially applying rule's loads are its items");
});

add_task(async function test_TestManifestConditions() {
  await TestUtils.loadPath("/tests/source/tests/test_otherthing.html");
  const box = frame.contentDocument.querySelector("section.info-boxes");
  const section = box.querySelector("a.test-manifest-section");
  is(section?.getAttribute("href"), "/tests/source/tests/mochitest-common.toml#3",
     "The manifest goes to the test's section");
  const conditions = [...box.querySelectorAll("a.test-skip-info")]
    .map(a => [a.textContent, a.getAttribute("href")]);
  is(JSON.stringify(conditions), JSON.stringify([
    ["verify", "/tests/source/tests/mochitest.toml#27"],
    ["os == 'android'", "/tests/source/tests/mochitest-common.toml#7"],
    ["asan", "/tests/source/tests/mochitest-common.toml#8"],
    ["os = 'mac'", "/tests/source/tests/mochitest-common.toml#4"],
    ["debug", "/tests/source/tests/mochitest-common.toml#5"],
  ]), "Each condition goes to its line, the including manifest's too");
  const comment = box.querySelector(".test-condition-comment a");
  is(comment?.getAttribute("href"), "https://bugzilla.mozilla.org/show_bug.cgi?id=1234",
     "A condition's comment's bug is a link");
});
