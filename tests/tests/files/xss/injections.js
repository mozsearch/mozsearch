/* Any copyright is dedicated to the Public Domain.
 * http://creativecommons.org/publicdomain/zero/1.0/ */

// Strings, keys and comments whose text is markup, none of which may become
// markup in any page showing it (see tests/webtest/test_XssEscaping.js).

// (A string a symbol's pretty name is, in the page's JSON data.)
var url = "chrome://a/</script><img id=xss-sym-info src=x>";

// (A key, the context of the uses in its function, in search results.)
var o = {
  "<img id=xss-context src=x>": function () {
    xssTarget();
  },
};

// javascript://%0aalert(1)
// moz-src:///x"data-xss-moz-src="1

function xssTarget() {}
