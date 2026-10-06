/**
 * /query/'s results (see query_results/*.liquid) are nested <details>: path
 * kinds, kinds (ex: "Definitions (Foo)"), and files.  Clicking anywhere in a
 * summary would toggle it, so only its disclosure triangle, a file's type
 * icon, and the name of a path kind or kind (`.query-toggle`) do, and links in
 * summaries (ex: a file's path) just work as links.
 */

/**
 * Clicks on the triangle, which is the summary's marker, target the summary
 * itself, but so do clicks beside the summary's contents (its headings are
 * inline blocks, so the rest of the row is the summary's), so only clicks
 * before the contents count, or above them, if they wrapped below the
 * triangle.
 */
function clickedMarker(summary, event) {
  const contents = summary.firstElementChild;
  if (!contents) {
    return true;
  }
  const rect = contents.getBoundingClientRect();
  const marginTop = parseFloat(getComputedStyle(contents).marginTop) || 0;
  return event.clientX < rect.left || event.clientY < rect.top - marginTop;
}

document.addEventListener("click", event => {
  const summary = event.target.closest?.(".query-result summary");
  if (!summary) {
    return;
  }
  if (event.target === summary) {
    // (Keyboard activation, without a position, always toggles.)
    if (event.detail === 0 || clickedMarker(summary, event)) {
      return;
    }
  } else if (event.target.closest(".query-toggle, .mimetype-bullet, a")) {
    return;
  }
  event.preventDefault();
});
