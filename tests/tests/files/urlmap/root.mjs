import { chrome3 } from "chrome://global/content/test/chrome3.mjs";
import { resource3 } from "resource://test/resource3.mjs";
import { mozsrc3 } from "moz-src:///urlmap/mozsrc3.mjs";

const { chrome1 } = ChromeUtils.importESModule("chrome://global/content/test/chrome1.mjs");
const { resource1 } = ChromeUtils.importESModule("resource://test/resource1.mjs");
const { mozsrc1 } = ChromeUtils.importESModule("moz-src:///urlmap/mozsrc1.mjs");

const lazy = {};
ChromeUtils.defineESModuleGetters(lazy, {
  chrome2: "chrome://global/content/test/chrome2.mjs",
  resource2: "resource://test/resource2.mjs",
  mozsrc2: "moz-src:///urlmap/mozsrc2.mjs",
});

window.open("chrome://global/content/test/chrome1.html");
window.open("resource://test/resource1.html");
window.open("moz-src:///urlmap/mozsrc1.html");
