/*---
description: A stub of Test262's array comparison, which tests include.
defines: [compareArray]
---*/
function compareArray(a, b) {
  return a.length === b.length && a.every((value, i) => value === b[i]);
}
