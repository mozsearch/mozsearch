// A Node script (CommonJS), which Node runs as a function's body, so that it
// can return before the rest of it.
const nodeScriptArgs = process.argv;
if (nodeScriptArgs.includes("-h")) {
  console.log("usage: node node-script.js");
  return;
}
function nodeScriptMain() {
  return nodeScriptArgs.length;
}
nodeScriptMain();
