/**
 * chrome://global/content/test/chrome1.mjs
 * https://bugzilla.mozilla.org/
 * resource://test/resource1.mjs
 * moz-src:///urlmap/mozsrc1.mjs
 */

// chrome://global/content/test/chrome1.css
// https://bugzilla.mozilla.org/
// resource://test/resource1.png
// moz-src:///urlmap/mozsrc1.mjs

void f() {
  const char* s = "chrome://global/content/test/chrome1.mjs";
  const char* t = "https://bugzilla.mozilla.org/";
  const char* u = "resource://test/resource1.png";
  const char* v = "moz-src:///urlmap/mozsrc1.mjs";
}

