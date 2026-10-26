/**
 * The patch stack strip (see tools/src/hyperblame/stack_strip.rs): a file's
 * view at a revision, opened from an explore page or a try push's page, whose
 * links pass the page's stack (ex: `?stack=explore/bug/123` or
 * `?stack=try/hg/abcdef012345`), shows a slot per commit of the stack beside each
 * line's blame strip, filled if the commit touched the line's tokens (in any of
 * the stack's states of the file), or scarred where it removed tokens, like the
 * explore page's sparklines per line.  Hovering over a slot names its commit
 * and highlights the commit's slots on the other lines, and a note above the
 * file links back to the stack's page.
 *
 * The stack's pages' links within the tree (to files and directories at
 * revisions, commits, outline diffs, canvases), and the links of pages in
 * its context, pass it on as they're followed, so that it stays with what's
 * browsed from them (ex: an outline row's link to a changed definition, a
 * directory listing's files).  (And the outline's default baseline in its
 * context is its base; see outline.js.)
 */
(function () {
  const STACK = /^(explore\/(bug|phab)\/[\w,]+|try\/hg\/[0-9a-f]+)$/i;
  const tree = location.pathname.split("/")[1];
  const param = new URLSearchParams(location.search).get("stack");
  // (The page's stack: its parameter's, or the page's own, if it's a stack's.)
  const own = location.pathname.split("/").slice(2).filter(Boolean).join("/");
  const stack = param && STACK.test(param) ? param : STACK.test(own) ? own : null;
  if (!tree || !stack) {
    return;
  }
  const KEEP = new RegExp(`^/${tree}/(rev|commit|outline-diff|rev-canvas)/`);
  const keep = event => {
    const link = event.target.closest?.("a[href]");
    if (!link) {
      return;
    }
    const url = new URL(link.href, location.href);
    if (url.origin != location.origin || !KEEP.test(url.pathname) || url.searchParams.has("stack")) {
      return;
    }
    // (As the stack's pages' own links have it, unescaped.)
    url.search += `${url.search ? "&" : "?"}stack=${stack}`;
    link.href = url.toString();
  };
  // (Before their clicks, middle clicks, context menus and keyboard use.)
  document.addEventListener("mousedown", keep, true);
  document.addEventListener("focusin", keep, true);

  const match = location.pathname.match(/^\/([^/]+)\/rev\/([0-9a-f]+)\/(.+)$/);
  if (!param || !match) {
    return;
  }
  const [, , rev, path] = match;

  function show(strip) {
    const count = strip.commits.length;
    if (!count) {
      return;
    }
    // (Narrower slots for bigger stacks.)
    const slot = count <= 8 ? 4 : count <= 24 ? 3 : 2;
    const root = document.documentElement;
    root.style.setProperty("--stack-slots", count);
    root.style.setProperty("--stack-slot", `${slot}px`);
    root.classList.add("stack-strips");

    const place = (line, slots, scar) => {
      const cell = document.querySelector(`#line-${line + 1} .blame-strip`)?.parentElement;
      if (!cell) {
        return;
      }
      let lineStrip = cell.querySelector(".stack-strip");
      if (!lineStrip) {
        lineStrip = document.createElement("div");
        lineStrip.className = "stack-strip";
        cell.append(lineStrip);
      }
      for (const k of slots) {
        const mark = document.createElement("span");
        mark.className = scar ? "stack-slot stack-scar" : "stack-slot";
        mark.dataset.slot = k;
        mark.style.left = `${k * slot}px`;
        mark.title = `${scar ? "Removed tokens here" : "Changed this line"}: ${k + 1}. ${strip.commits[k][1]}`;
        lineStrip.append(mark);
      }
    };
    for (const [line, slots] of strip.lines) {
      place(line, slots, false);
    }
    for (const [line, slots] of strip.scars) {
      place(line, slots, true);
    }

    // Hovering over a slot highlights its commit's slots.
    let hot = null;
    document.addEventListener("mouseover", event => {
      const k = event.target.closest?.(".stack-slot")?.dataset.slot ?? null;
      if (k === hot) {
        return;
      }
      hot = k;
      for (const mark of document.querySelectorAll(".stack-slot.stack-hot")) {
        mark.classList.remove("stack-hot");
      }
      if (k !== null) {
        for (const mark of document.querySelectorAll(`.stack-slot[data-slot="${k}"]`)) {
          mark.classList.add("stack-hot");
        }
      }
    });

    const note = document.createElement("div");
    note.className = "stack-strip-note";
    const link = document.createElement("a");
    link.href = `/${tree}/${stack}`;
    link.textContent = `the ${count} commit${count == 1 ? "" : "s"} of ${stack.startsWith("try/") ? "the try push" : "the stack"}`;
    note.append("Beside the blame, which of ", link,
                " changed each line (hover over a slot to see which).");
    document.getElementById("file")?.before(note);
    // (For change-nav.js, which navigates between the changed lines.)
    document.dispatchEvent(new CustomEvent("stack-strip-drawn", { detail: strip }));
  }

  // (Shared with outline.js, whose default baseline is the stack's base.)
  (window.stackStripData ??= fetch(`/${tree}/${stack}/stack-strip/${rev}/${path}`)
    .then(response => (response.ok ? response.json() : null))
    .catch(() => null))
    .then(strip => strip && show(strip));
})();
