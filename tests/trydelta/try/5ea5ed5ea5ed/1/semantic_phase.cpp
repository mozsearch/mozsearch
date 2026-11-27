// A function of a push whose semantic analysis waits a while (see its
// slow-semantic and ../../import-try), for test_TrySemanticPhase.js to see
// its review while it's analyzing.
namespace trydelta {
int SemanticPhase() { return 42; }
}  // namespace trydelta
