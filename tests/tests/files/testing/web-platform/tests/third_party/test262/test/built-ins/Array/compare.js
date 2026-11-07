// A Test262 test, which web-platform-tests runs after its harness's
// assert.js and sta.js, and its frontmatter's includes.
/*---
description: An include's function
includes: [compareArray.js]
---*/
assert(compareArray([1, 2], [1, 2]), "The arrays are the same");
assert.sameValue([1, 2].length, 2);
