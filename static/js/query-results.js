/**
 * /query/'s results (see query_results/*.liquid) are nested <details>: path
 * kinds, kinds (ex: "Definitions (Foo)"), and files.  Clicking anywhere in a
 * summary would toggle it, so only its disclosure triangle, a file's type
 * icon, and the name of a path kind or kind (`.query-toggle`) do, and links
 * and labels in summaries (ex: a file's path) just work.
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
  } else if (event.target.closest(".query-toggle, .mimetype-bullet, a, label")) {
    return;
  }
  event.preventDefault();
});

/**
 * Textual occurrences on lines that other results already show (for their
 * other matches) are hidden unless the checkboxes in the textual occurrences'
 * headings say to include them, which we remember.
 */
const SHOW_REPEATED_KEY = "query-show-repeated";

function showRepeated(show) {
  document.querySelector(".query-result")?.classList.toggle("query-show-repeated", show);
  for (const checkbox of document.querySelectorAll(".query-repeated-toggle input")) {
    checkbox.checked = show;
  }
}

showRepeated(localStorage.getItem(SHOW_REPEATED_KEY) == "1");
document.addEventListener("change", event => {
  if (event.target.matches?.(".query-repeated-toggle input")) {
    localStorage.setItem(SHOW_REPEATED_KEY, event.target.checked ? "1" : "0");
    showRepeated(event.target.checked);
  }
});
