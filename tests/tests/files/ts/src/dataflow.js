// Fixtures for following values (see tools/src/dataflow/ and the checks in
// tests/tests/checks/inputs/dataflow/): locals, parameters, properties, and
// an XPIDL method (`tsMethod`, see bindings.d.ts) whose C++ implementation is
// in dataflow/dataflow.cpp.

function pick(flag, fallback) {
  const choice = flag ? "on" : fallback;
  return choice;
}

function describe(settings) {
  const label = pick(settings.enabled, "off");
  settings.label = label;
  return label;
}

function viaXpcom(input) {
  const result = tsMethod(input, false);
  if (result) {
    return describe({ enabled: result });
  }
  return null;
}
