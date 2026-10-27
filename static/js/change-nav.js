/**
 * Navigating between a page's changes: on diffs (`/diff/`, `/interdiff/`) and
 * on files' views in a stack's context (`?stack=`, ex: a try push's, whose
 * changed lines stack-strip.js marks), the panel's "Changes" section has
 * buttons for the previous and next change ("k" and "j", as accelerators),
 * which scroll to it and mark its rows, and a choice of granularity:
 * - "token": each run of changed rows (or of changed tokens, on interdiffs);
 * - "para": each paragraph with changes, its runs grown to the blank lines
 *   around them (the default);
 * - "section": each section with changes, grown to the headings around them
 *   (HTML's, Wattsi's, ecmarkup's or Bikeshed's), or in files without any, to
 *   the top-level lines around them (ex: a function's definition).
 * These are approximations from the rows' text, which is all a diff has.
 * The granularity is remembered.
 *
 * Pages whose changes aren't rows give their own instead, as a function of
 * them (in the page's order, each's element and the elements it marks), in a
 * `change-nav-targets` event, without granularities (ex: a spec's rendered
 * page with its pull requests' changes beside it; see spec-pr-view.js).
 */
(function () {
  const GRANULARITIES = [
    ["token", "Each run of changed rows"],
    ["para", "Each paragraph with changes"],
    ["section", "Each section with changes"],
  ];
  const STORAGE = storageKey("change-nav-granularity");
  const HEADING = /^\s*(<h[1-6][\s>]|<emu-clause[\s>]|<section[\s>]|#{1,6}\s)/i;
  const TOP_LEVEL = /^[^\s}\])#/*]/;

  const rows = () => [...document.querySelectorAll("#file div[role=row]")];
  // (A row's text, without a diff's origin column: a character per parent and
  // a space, ex: "+ ", as its changed rows show.)
  let originWidth = null;
  const origin = () => (originWidth ??= document.querySelector("#file code.plus-line, #file code.minus-line")
    ?.textContent.match(/^[+\- ]*[+\-] /)?.[0].length ?? 0);
  const text = row => (row.querySelector("code")?.textContent || "").slice(origin());
  const blank = row => text(row).trim() == "";

  // The rows a diff or interdiff changes.
  function diffChanged(row) {
    const code = row.querySelector("code");
    return !!code && (code.classList.contains("plus-line") || code.classList.contains("minus-line") ||
                      row.classList.contains("interdiff-only-a") ||
                      !!code.querySelector(".idiff-new, .idiff-kept, .idiff-rm-new, .idiff-only-a"));
  }

  // The rows a stack changed, from the stack strip's data (see stack-strip.js).
  let stackLines = null;

  function changed() {
    const all = rows();
    if (stackLines) {
      return all.map(row => stackLines.has(row.id));
    }
    return all.map(diffChanged);
  }

  // The units of the granularity, as [first, last] indices of rows.
  function units(granularity) {
    const all = rows();
    const marks = changed();
    const runs = [];
    for (let i = 0; i < all.length; i++) {
      if (marks[i] && (i == 0 || !marks[i - 1])) {
        runs.push([i, i]);
      } else if (marks[i]) {
        runs[runs.length - 1][1] = i;
      }
    }
    if (granularity == "token") {
      return runs;
    }
    let isBoundary;
    if (granularity == "para") {
      isBoundary = i => blank(all[i]);
    } else {
      const heading = i => HEADING.test(text(all[i]));
      const anyHeading = all.some((_, i) => heading(i));
      isBoundary = anyHeading ? heading : i => TOP_LEVEL.test(text(all[i]));
    }
    // (A paragraph is between blank rows, which it doesn't include; a section
    // starts at its heading.)
    const grown = runs.map(([first, last]) => {
      let start = first;
      while (start > 0 && !(granularity == "para" ? isBoundary(start - 1) : isBoundary(start))) {
        start--;
      }
      let end = last;
      while (end + 1 < all.length && !isBoundary(end + 1)) {
        end++;
      }
      return [start, end];
    });
    const merged = [];
    for (const unit of grown) {
      const previous = merged[merged.length - 1];
      if (previous && unit[0] <= previous[1] + 1) {
        previous[1] = Math.max(previous[1], unit[1]);
      } else {
        merged.push(unit);
      }
    }
    return merged;
  }

  let section = null;
  let status = null;
  let current = -1;
  let cached = null;
  // (The page's own changes, if it gives them.)
  let targets = null;
  const granularity = () => {
    const stored = localStorage.getItem(STORAGE);
    return GRANULARITIES.some(([key]) => key == stored) ? stored : "para";
  };
  const currentUnits = () => (targets ? targets() : (cached ??= units(granularity())));

  function mark(index) {
    for (const row of document.querySelectorAll(".change-nav-current")) {
      row.classList.remove("change-nav-current");
    }
    const unit = currentUnits()[index];
    if (!unit) {
      return;
    }
    if (targets) {
      for (const marked of [unit.element, ...unit.marked]) {
        marked.classList.add("change-nav-current");
      }
      unit.element.scrollIntoView({ block: "center" });
      return;
    }
    const all = rows();
    for (let i = unit[0]; i <= unit[1]; i++) {
      all[i].classList.add("change-nav-current");
    }
    all[unit[0]].scrollIntoView({ block: unit[1] - unit[0] > 20 ? "start" : "center" });
  }

  function updateStatus() {
    const count = currentUnits().length;
    const noun = targets ? "change" : { token: "change", para: "paragraph", section: "section" }[granularity()];
    const plural = `${count} ${noun}${count == 1 ? "" : "s"}`;
    status.textContent = current >= 0 ? `${current + 1} of ${plural}` : plural;
  }

  // The next (or previous) unit: from the marked one if it's still in view,
  // else the first one from the top of the view (or the last one above it).
  function step(direction) {
    const list = currentUnits();
    if (!list.length) {
      return;
    }
    if (targets) {
      // (Whose order changes as they're filtered.)
      current = list.findIndex(unit => unit.element.classList.contains("change-nav-current"));
    }
    // (A unit's first element.)
    const all = targets ? null : rows();
    const element = index => (targets ? list[index].element : all[list[index][0]]);
    const top = index => element(index).getBoundingClientRect().top;
    const inView = index => {
      const rect = element(index).getBoundingClientRect();
      return rect.bottom > 0 && rect.top < window.innerHeight;
    };
    let next;
    if (current >= 0 && current < list.length && inView(current)) {
      next = Math.min(list.length - 1, Math.max(0, current + direction));
    } else {
      // (Below the header, which the page scrolls under.)
      const viewTop = (document.getElementById("scrolling")?.getBoundingClientRect().top || 0) + 40;
      const indices = list.map((_, i) => i);
      next = direction > 0
        ? indices.find(i => top(i) >= viewTop) ?? list.length - 1
        : indices.findLast(i => top(i) < viewTop) ?? 0;
    }
    current = next;
    mark(current);
    updateStatus();
  }

  function setGranularity(key) {
    localStorage.setItem(STORAGE, key);
    cached = null;
    current = -1;
    for (const row of document.querySelectorAll(".change-nav-current")) {
      row.classList.remove("change-nav-current");
    }
    for (const button of section.querySelectorAll(".change-nav-granularity button")) {
      button.setAttribute("aria-pressed", button.dataset.granularity == key ? "true" : "false");
    }
    updateStatus();
  }

  function show() {
    const content = document.getElementById("panel-content");
    if (section || !content) {
      return;
    }
    section = document.createElement("div");
    section.className = "change-nav";
    const heading = document.createElement("h4");
    heading.textContent = "Changes";
    const list = document.createElement("ul");
    const item = (label, accel, title, direction) => {
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.className = "icon item";
      button.dataset.accel = accel;
      button.title = title;
      const key = document.createElement("span");
      key.className = "accel";
      key.textContent = accel;
      if (!Panel.acceleratorsEnabled) {
        key.style.display = "none";
      }
      button.append(label, " ", key);
      button.addEventListener("click", () => step(direction));
      li.append(button);
      return li;
    };
    const controls = document.createElement("li");
    controls.className = "change-nav-controls";
    status = document.createElement("span");
    status.className = "change-nav-status";
    const modes = document.createElement("span");
    modes.className = "lens-modes change-nav-granularity";
    modes.setAttribute("role", "group");
    modes.setAttribute("aria-label", "Granularity");
    for (const [key, title] of GRANULARITIES) {
      const button = document.createElement("button");
      button.type = "button";
      button.dataset.granularity = key;
      button.title = title;
      button.textContent = key;
      button.addEventListener("click", () => setGranularity(key));
      modes.append(button);
    }
    // (A page's own changes have no granularities.)
    modes.hidden = !!targets;
    controls.append(status, modes);
    list.append(
      item("Previous change", "K", "Go to the previous change", -1),
      item("Next change", "J", "Go to the next change", 1),
      controls,
    );
    section.append(heading, list);
    // (At the top, after the lenses, if any.)
    const lenses = content.querySelector(":scope > .lens-bar");
    if (lenses) {
      lenses.after(section);
    } else {
      content.prepend(section);
    }
    setGranularity(granularity());
  }

  // Diffs have their changes at once; stack contexts once the stack strip has
  // its data.
  if (document.querySelector("#content.diff, .source-listing.diff") && rows().some(diffChanged)) {
    show();
  }
  document.addEventListener("change-nav-targets", event => {
    targets = event.detail;
    current = -1;
    show();
    section.querySelector(".change-nav-granularity").hidden = true;
    updateStatus();
  });
  document.addEventListener("stack-strip-drawn", event => {
    stackLines = new Set([...event.detail.lines, ...event.detail.scars].map(([line]) => `line-${line + 1}`));
    cached = null;
    if (stackLines.size) {
      show();
      updateStatus();
    }
  });
})();
