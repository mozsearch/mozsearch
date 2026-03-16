"use strict";

add_task(async function test_Hoist() {
  await TestUtils.loadPath("/tests/source/js/local-decl-hoist.js");

  {
    const use = frame.contentDocument.querySelector(`#line-2 span[data-symbols]`);
    const def = frame.contentDocument.querySelector(`#line-4 span[data-symbols]`);
    is(use.getAttribute("data-symbols"),
       def.getAttribute("data-symbols"),
       "Function should be found");
  }

  {
    const use = frame.contentDocument.querySelector(`#line-18 span[data-symbols]`);
    const def = frame.contentDocument.querySelector(`#line-12 span[data-symbols]`);
    is(use.getAttribute("data-symbols"),
       def.getAttribute("data-symbols"),
       "Function should be found");
  }

  {
    const use = frame.contentDocument.querySelector(`#line-31 span[data-symbols]`);
    const def1 = frame.contentDocument.querySelector(`#line-22 span[data-symbols]`);
    const def2 = frame.contentDocument.querySelector(`#line-25 span[data-symbols]`);
    is(use.getAttribute("data-symbols"),
       def1.getAttribute("data-symbols"),
       "Variable should be found");
    is(use.getAttribute("data-symbols"),
       def2.getAttribute("data-symbols"),
       "Function should also be found");
  }

  {
    const use = frame.contentDocument.querySelector(`#line-44 span[data-symbols]`);
    const def1 = frame.contentDocument.querySelector(`#line-35 span[data-symbols]`);
    const def2 = frame.contentDocument.querySelector(`#line-38 span[data-symbols]`);
    is(use.getAttribute("data-symbols"),
       def1.getAttribute("data-symbols"),
       "Variable should be found");
    isnot(use.getAttribute("data-symbols"),
          def2.getAttribute("data-symbols"),
          "Function should not be found");
  }

  {
    const use = frame.contentDocument.querySelector(`#line-48 span[data-symbols]`);
    const def = frame.contentDocument.querySelector(`#line-51 span[data-symbols]`);
    is(use.getAttribute("data-symbols"),
       def.getAttribute("data-symbols"),
       "Function should be found");
  }
});
