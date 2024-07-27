"use strict";

add_task(async function test_FieldLayoutForGenerated() {
  const sym = "generated::GeneratedStruct";

  await TestUtils.loadPath(`/tests/query/default?q=field-layout:'${sym}'`);

  const symInLine = document.querySelectorAll(`span.syn_def[data-symbols="F_<T_generated::GeneratedStruct>_x"]`);
  ok(!!symInLine, "Symbol in the definition line exists");
});
