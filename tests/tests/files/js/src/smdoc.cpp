// SpiderMonkey documents its concepts in [SMDOC] comment blocks; see
// tools/languages/semantic_mappings/smdoc_cpp.toml.

// [SMDOC] Test Concept (with context)
//
// A concept that other comments cite.

/*
 * [SMDOC] Another Concept
 *
 * Its details.
 */

namespace js {

// See [SMDOC] Test Concept for how this works, and [SMDOC] Another Concept.
int CitesConcepts() { return 0; }

}  // namespace js
