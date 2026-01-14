"use strict";

add_task(async function test_DiagramLambdaInFunc() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'lambda_in_func::func' depth:4");

  const nodeSyms = [
    "_ZN14lambda_in_func7caller1Ev",
    "_ZN14lambda_in_func5test1Ev",
    "_ZZN14lambda_in_func5test1EvENK3$_0clEv",
    "_ZN14lambda_in_func4funcEv",
  ];

  for (const sym of nodeSyms) {
    const node = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(node, `${sym} node exists`);
  }

  const edgeSyms = [
    "_ZN14lambda_in_func7caller1Ev->_ZN14lambda_in_func5test1Ev",
    "_ZN14lambda_in_func5test1Ev->_ZZN14lambda_in_func5test1EvENK3$_0clEv",
    "_ZZN14lambda_in_func5test1EvENK3$_0clEv->_ZN14lambda_in_func4funcEv",
  ];

  for (const sym of edgeSyms) {
    const edge = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(edge, `${sym} edge exists`);
  }
});

add_task(async function test_DiagramLambdaInMethod() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'lambda_in_method::func' depth:4");

  const nodeSyms = [
    "_ZN16lambda_in_method7caller1Ev",
    "T_lambda_in_method::C",
    "_ZN16lambda_in_method1C2m0Ev",
    "_ZZN16lambda_in_method1C2m0EvENKUlvE_clEv",
    "_ZN16lambda_in_method4funcEv",
  ];

  for (const sym of nodeSyms) {
    const node = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(node, `${sym} node exists`);
  }

  const edgeSyms = [
    "T_lambda_in_method::C:_ZN16lambda_in_method1C2m0Ev->_ZZN16lambda_in_method1C2m0EvENKUlvE_clEv",
    "_ZZN16lambda_in_method1C2m0EvENKUlvE_clEv->_ZN16lambda_in_method4funcEv",
    "_ZN16lambda_in_method7caller1Ev->T_lambda_in_method::C:_ZN16lambda_in_method1C2m0Ev",
  ];

  for (const sym of edgeSyms) {
    const edge = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(edge, `${sym} edge exists`);
  }
});
