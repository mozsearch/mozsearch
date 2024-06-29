# Webtest

Webtest is a testing command for the searchfox UI part.

## Running tests

Webtest can be run by the following command inside the docker image.

```
make webtest
```

This does the following:
  * Build the test repo
  * Install geckodriver
  * Download Firefox
  * Start geckodriver
  * Run the `searchfox-tools`s `webtest` command, which loads and runs the tests in headless browser
  * Stop geckodriver

If the test repo is ready, you can also directly run `./scripts/webtest.sh`, which does the above, excluding the build for the test repo.

```
./scripts/webtest.sh
```

`webtest.sh` takes an optional filter arguments, to specify which test to run.
It's a substring-match on the test path.

```
./scripts/webtest.sh FILTER

# examples
./scripts/webtest.sh tests/webtest/test_Search.js
./scripts/webtest.sh test_Search.js
./scripts/webtest.sh Search
```

## Structure

Webtest consists of the following parts:
  * [Firefox](https://www.mozilla.org/en-US/firefox/) (automatically downloaded into `/vagrant/mozsearch-firefox`)
  * [geckodriver](https://github.com/mozilla/geckodriver) (automatically installed)
  * `searchfox-tool` command `webtest`: Command to initiate the test and analyze logs
  * `tests/webtest/webtest.html`: The top-level frame for the test, loaded in the browser
  * `tests/webtest/head.js`: A test harness and utility, loaded into `webtest.html`

## Test files

Test files should be put inside `tests/webtest/` directory, and named `test_*.js`.

Simple test file looks like the following:

```js
"use strict";

add_task(async function test_Header() {
  await TestUtils.loadPath("/");

  const h1 = frame.contentDocument.querySelector("h1");

  is(h1.textContent, "Welcome to Searchfox [testing]",
    "The header is there");
});
```

The test file is loaded into the `webtest.html`'s top-level frame.
`webtest.html` has an iframe for loading searchfox page, and `frame` global variable
holds the reference to it.

## Global variables

Test harness and utility functions are made similar to mochitest-browser style,
in order to make it easier for people who's familiar with the mozilla-central code.

Functions provided by the test harness:

  * `add_test(func)`: add single subtest, with as async function
  * `registerCleanupFunction(func)`: add a function to perfom at the end of the current subtest

Assertion functions:

  * `ok(condition, "Description of the check")`: tests a value for its truthfulness
  * `is(actual, expected, "Description of the check")`:  compares two values (using `Object.is`)
  * `isnot(actual, expected, "Description of the check")`: opposite of `is()`
  * `waitForCondition(condition, "Description of the check")`: wait until `condition` becomes true

Utility functions:

  * `TestUtils.loadPage(path)`: load the specified path of searchfox page into the iframe
  * `TestUtils.shortenSearchTimeouts()`: Shorten the timeout value for query and history, in order to shorten the time taken by tests
  * `TestUtils.setFeatureGate(name, value)`: set the feature gate value. e.g. `TestUtils.setFeatureGate("semanticInfo", "release")`  (this opens the settings page)
  * `TestUtils.resetFeatureGate(name)`: undo `TestUtils.setFeatureGate`

Other global variables:

  * `frame`: the iframe which loads the searchfox page

Misc:

  * `info(msg)`: print "INFO" message

For the complete list, please see `head.js`.

## Running from browser

In case you want to debug the test, webtest can also be run from the browser.

  1. Build the test repo (`make build-test-repo`)
  2. Open `http://localhost:16995/tests/webtest/webtest.html` (The private browsing mode is recommended, in order to avoid interferring with the settings)
  3. Open console
  4. Run `TestHarness.loadTest(TES_PATH);`.  e.g. `TestHarness.loadTest("tests/webtest/test_Search.js")`

The log is printed to the console.
