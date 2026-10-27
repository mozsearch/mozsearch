/**
 * A note atop a file's or directory's page saying that a link to where it was
 * (a path the tree doesn't have anymore) brought one here, where it moved
 * (see `format::format_missing_source`): from its `moved-from` parameter, the
 * path, and its `moved-in` parameter, the commit which moved it (none for a
 * directory, most of whose files moved here).
 */
(function () {
  const params = new URLSearchParams(location.search);
  const from = params.get("moved-from");
  if (!from) {
    return;
  }
  const tree = location.pathname.split("/")[1];
  const rev = params.get("moved-in") || "";
  const note = document.createElement("div");
  note.className = "redirect-note";
  const path = document.createElement("code");
  path.textContent = from;
  note.append("Redirected from ", path, ", which moved here");
  if (/^[0-9a-f]{40}$/.test(rev)) {
    const link = document.createElement("a");
    link.href = `/${tree}/commit/${rev}`;
    link.textContent = rev.slice(0, 12);
    note.append(" in ", link);
  }
  // (A link's line, in its fragment, may be another line here.)
  note.append(location.hash ? "; its lines may be numbered differently here." : ".");
  const anchor = document.getElementById("file") || document.querySelector(".folder-content");
  if (anchor) {
    anchor.before(note);
  } else {
    document.getElementById("content")?.prepend(note);
  }
})();
