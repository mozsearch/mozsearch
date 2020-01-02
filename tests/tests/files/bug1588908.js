/**
 * This tests the regexp literal arrow function from
 * BrowserTestUtils.waitForDocLoadAndStopIt, which was breaking syntax
 * highlighting in bug 1588908.
 */

let isHttp = url => /^https?:/.test(url);

function f(x) {
  // Not sure why anyone would ever do this, but it's valid JavaScript...
  return x < /^https?:/;
}
