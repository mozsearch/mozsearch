"use strict";

// C++ locals' and parameters' symbols have crossref entries (see
// tools/src/file_format/local_symbols.rs), so their context menus jump and
// search like other symbols', including across files: a declaration's
// parameter jumps to the definition's, and a generated IPDL method's
// parameter to its message's parameter.

function findMenuLink(menu, text) {
  return [...menu.querySelectorAll(".contextmenu-link")].find(
    link => link.textContent === text);
}

async function openMenuFor(line, text) {
  const token = [...frame.contentDocument.querySelectorAll(`#line-${line} span[data-symbols]`)]
    .find(span => span.textContent === text);
  ok(token, `The token ${text} is on line ${line}`);
  TestUtils.click(token);

  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, `Context menu is shown for ${text}`);
  return menu;
}

add_task(async function test_ParameterDeclarationJumpsToDefinition() {
  await TestUtils.loadPath("/tests/source/params/params.h");

  // `int Scale(int aValue, int aFactor);`
  const menu = await openMenuFor(7, "aFactor");

  const def = findMenuLink(menu, "Go to definition of params::Scale::aFactor");
  ok(def, "The declaration's parameter has a jump to the definition's");
  ok(def?.href.endsWith("/tests/source/params/params.cpp#7"),
     `The jump is to the definition in the .cpp: ${def?.href}`);
  ok(findMenuLink(menu, "Search for params::Scale::aFactor"),
     "The parameter has a search");
});

add_task(async function test_LocalJumpsToDefinition() {
  await TestUtils.loadPath("/tests/source/params/params.cpp");

  // `return scaled;`
  const menu = await openMenuFor(15, "scaled");

  const def = findMenuLink(menu, "Go to definition of params::UseThem::scaled");
  ok(def, "The local's use has a jump to its definition");
  ok(def?.href.endsWith("/tests/source/params/params.cpp#14"),
     `The jump is to its declaration's line: ${def?.href}`);
});

add_task(async function test_IpdlParameterSlot() {
  await TestUtils.loadPath(
    "/tests/source/__GENERATED__/ipc/ipdl/_ipdlheaders/mozilla/_ipdltest/PTestDataStructuresChild.h");

  // `const SIntDouble& i,` (SendTestStruct's)
  const menu = await openMenuFor(226, "i");

  const idl = findMenuLink(
    menu, "Go to IDL definition of mozilla__ipdltest::PTestDataStructures::TestStruct::i");
  ok(idl, "The Send method's parameter has a jump to its message's parameter");
  ok(idl?.href.endsWith("/tests/source/ipdl/PTestDataStructures.ipdl#70"),
     `The jump is to the .ipdl: ${idl?.href}`);
});
