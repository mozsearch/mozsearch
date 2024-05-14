const { chrome1 } = ChromeUtils.importESModule("chrome://global/content/test/chrome1.mjs");
const { resource1 } = ChromeUtils.importESModule("resource://test/resource1.mjs");

const lazy = {};
ChromeUtils.defineESModuleGetters(lazy, {
  chrome2: "chrome://global/content/test/chrome2.mjs",
  resource2: "resource://test/resource2.mjs",
});

window.open("chrome://global/content/test/chrome1.html");
window.open("resource://test/resource1.html");
