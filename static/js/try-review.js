/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A try push's review view (the `try-review:` query term's; see the tools'
// try_review.rs): its parts' review marks (see review-state.js), kept in the
// browser and shared with its other tabs, the review's summary in the
// navigation panel (what's marked how, and that it's kept), the view's crumbs
// in the page's breadcrumbs, and its sections and patches hidden when the
// panel's facets hide all of their files.

// (A menu's groups in `count` columns of about as many rows.)
function menuColumns(groups, count) {
  const rows = groups.reduce((n, group) => n + group.items.length + 1, 0);
  const perColumn = Math.ceil(rows / count);
  const out = [[]];
  let inColumn = 0;
  for (const group of groups) {
    if (inColumn > 0 && inColumn + group.items.length + 1 > perColumn && out.length < count) {
      out.push([]);
      inColumn = 0;
    }
    out[out.length - 1].push(group);
    inColumn += group.items.length + 1;
  }
  return out;
}

// Fills a menu with its groups in as many columns as fit within `maxWidth`
// (at most 3, and about 16 rows each), or else in one, whose items' text is
// cut short (with their whole text as their titles).  The menu is laid out
// hidden first, and measured, so that it's shown at its size, never
// wrapping its items' icons and text apart nor resizing once it's shown.
function fillFittedMenu(menu, groups, maxWidth) {
  const element = menu.menu;
  const rows = groups.reduce((n, group) => n + group.items.length + 1, 0);
  const most = Math.min(3, Math.max(1, Math.ceil(rows / 16)));
  element.classList.remove("tr-menu-narrow");
  element.style.removeProperty("--tr-menu-width");
  element.style.visibility = "hidden";
  element.style.display = "flex";
  try {
    for (let count = most; count >= 1; count--) {
      fillCrashMenu(menu, menuColumns(groups, count));
      if (element.scrollWidth <= maxWidth || count == 1) {
        break;
      }
    }
    if (element.scrollWidth > maxWidth) {
      element.classList.add("tr-menu-narrow");
      element.style.setProperty("--tr-menu-width", `${Math.max(200, maxWidth)}px`);
      for (const link of element.querySelectorAll("a")) {
        link.title ||= link.textContent.trim();
      }
    }
  } finally {
    element.style.visibility = "";
  }
}

var TryReview = new (class TryReview {
  constructor() {
    this.root = document.getElementById("try-review");
    if (!this.root || typeof TRY_REVIEW == "undefined" || typeof ReviewState == "undefined") {
      return;
    }
    this.review = new ReviewState.Review(TRY_REVIEW.key, TRY_REVIEW.indices);
    for (const [id, parent, level, label] of TRY_REVIEW.parts) {
      this.review.addPart(id, parent, level, label);
    }
    this.moveCrumbs();
    this.addPatchPicker();
    this.addJumpPicker();
    this.addThenNowCrumb();
    this.addPanelSection();
    this.root.addEventListener("facetschange", () => this.hideEmpty());
    this.ready = this.start();
  }

  async start() {
    await this.review.open();
    // (Each part's strip, where the view left a slot for it.)
    for (const slot of this.root.querySelectorAll(".tr-strip[data-part]")) {
      slot.replaceWith(ReviewState.strip(this.review, slot.dataset.part));
    }
    // (And where its state matters more than changing it, ex: a patch's
    // heading, a button of its mark, which opens its strip.)
    for (const slot of this.root.querySelectorAll(".tr-state[data-part]")) {
      slot.replaceWith(ReviewState.stateButton(this.review, slot.dataset.part));
    }
    this.rememberLooking();
    this.addLinePopups();
    this.addLineHover();
    ReviewState.bind(this.review, this.root);
    this.drawSlicings();
    this.summaryBox.append(ReviewState.summary(this.review, [
      ["patch", "Patches"],
      ["file", "Files"],
      ["class", "Classes"],
      ["function", "Functions"],
      ["hunk", "Hunks"],
    ], part => this.go(part)));
    this.review.subscribe(() => this.updateStatus());
    this.updateStatus();
    this.focusRevision();
    this.root.dataset.reviewReady = "";
  }

  // A Phabricator revision's review URL (`/{tree}/review/phab/D123`, its
  // latest diff's push's review, or `.../D123/{push}`; see the web-server's
  // `review` route) starts at the revision's patch, which the push's stack
  // has below it, unless the URL has a fragment.
  focusRevision() {
    const match = location.pathname.match(/\/review\/phab\/D(\d+)(?:\/[0-9a-f]+)?\/?$/);
    if (!match || location.hash.length > 1) {
      return;
    }
    const patch = this.root.querySelector(`.tr-patch[data-phab="${match[1]}"]`);
    if (patch) {
      this.root.dataset.focusedPatch = patch.id;
      if (patch != this.root.querySelector(".tr-patch")) {
        patch.scrollIntoView({ block: "start" });
      }
    }
  }

  // (The view's trail, after the tree's, in the page's breadcrumbs.)
  moveCrumbs() {
    this.crumbs = this.root.querySelector(".tr-crumbs");
    const breadcrumbs = document.querySelector(".breadcrumbs");
    if (this.crumbs && breadcrumbs) {
      breadcrumbs.append(this.crumbs);
      this.crumbs.hidden = false;
    }
  }

  // The breadcrumbs' patch: the one scrolled to (see `updateCurrentPatch`),
  // with a picker of the patches by their groups (see the tools'
  // `group_patches`), each with its mark, a group's honorary members after
  // its own (as crash-view.js's thread picker has threads by their kinds).
  addPatchPicker() {
    this.patchLink = this.crumbs?.querySelector(".tr-crumb-patch-link");
    this.patchSections = [...this.root.querySelectorAll(".tr-patch")];
    const picker = document.getElementById("tr-patch-picker");
    if (!picker || typeof CrashMenu == "undefined") {
      return;
    }
    this.patchMenu = new CrashMenu(picker, document.getElementById("tr-patch-menu"),
                                   this.crumbs.querySelector(".tr-crumb-patch"),
                                   menu => this.fillPatchMenu(menu));
    const open = this.patchMenu.open.bind(this.patchMenu);
    this.patchMenu.open = () => {
      open();
      this.keepInView(this.patchMenu.menu);
    };
    let scheduled = false;
    window.addEventListener("scroll", () => {
      if (!scheduled) {
        scheduled = true;
        requestAnimationFrame(() => {
          scheduled = false;
          this.updateCurrentPatch();
        });
      }
    }, { passive: true });
    this.updateCurrentPatch();
  }

  fillPatchMenu(menu) {
    const patches = TRY_REVIEW.patches;
    const groups = TRY_REVIEW.groups?.length
      ? TRY_REVIEW.groups
      : patches.map((_, i) => ({ patches: [i], honorary: [] }));
    const item = (i, honorary) => new MenuItem({
      html: `<span class="tr-menu-mark">${this.icon(patches[i].part)}</span> ` +
            `<span class="tr-menu-patch">Patch ${i + 1}: ${escapeHtml(patches[i].summary)}</span>` +
            (honorary ? ` <span class="crash-menu-owner">(related)</span>` : ""),
      classNames: ["indent"].concat(honorary ? ["tr-menu-honorary"] : []),
      attrs: { "data-patch": String(i) },
      action: () => {
        menu.hide();
        this.showPatch(i);
      },
    });
    const sorted = groups.map((group, g) => ({
      label: groups.length > 1 ? `Group ${g + 1}` : null,
      detail: group.shared?.length ? group.shared.join(", ") : null,
      items: group.patches.map(i => item(i, false))
        .concat((group.honorary || []).map(i => item(i, true))),
    }));
    fillFittedMenu(menu, sorted, this.root.getBoundingClientRect().width - 16);
    return menu.menu.querySelector(`a[data-patch="${this.currentPatch ?? 0}"]:not(.tr-menu-honorary)`);
  }

  // (A picker's menu within the view, whose right the navigation panel can
  // cover, rather than the window; see `fillFittedMenu`.)
  keepInView(menu) {
    const over = menu.getBoundingClientRect().right - this.root.getBoundingClientRect().right;
    if (over > 0) {
      menu.style.left = Math.max(0, menu.offsetLeft - over) + "px";
    }
  }

  // (A part's mark's icon, as HTML: its own, or what it has from its
  // container.)
  icon(part) {
    return ReviewState.iconHtml(this.review.effective(part).state);
  }

  // The sections' jump picker: a popup, under the section's heading, of its
  // patch's files, classes and functions by section, each with its mark,
  // going to it.  (One menu, which each section's button opens.)
  addJumpPicker() {
    const menu = document.getElementById("tr-jump-menu");
    if (!menu || typeof CrashMenu == "undefined") {
      return;
    }
    const jumps = new CrashMenu(null, menu, null, () => this.fillJumpMenu(jumps));
    // (Its heading above the later ones while it's open, which would
    // otherwise cover it.)
    jumps.hide = () => {
      CrashMenu.prototype.hide.call(jumps);
      menu.parentElement?.classList.remove("tr-jumping");
    };
    this.root.addEventListener("click", event => {
      const button = event.target.closest(".tr-jump-picker");
      if (!button) {
        return;
      }
      const shown = jumps.isShown() && jumps.button == button;
      jumps.hide();
      if (shown) {
        return;
      }
      jumps.button = button;
      jumps.anchor = button;
      const head = button.closest(".tr-section-head");
      head.append(menu);
      head.classList.add("tr-jumping");
      jumps.open();
      this.keepInView(menu);
    });
  }

  fillJumpMenu(jumps) {
    const button = jumps.button;
    const here = button.closest(".tr-section");
    const patch = button.closest(".tr-patch");
    let focus = null;
    const groups = [...patch.querySelectorAll(".tr-section")].filter(section => !section.hidden).map(section => {
      const items = [];
      for (const file of section.querySelectorAll(".tr-file[data-part]")) {
        if (file.hidden) {
          continue;
        }
        const path = file.dataset.path ?? "";
        const slash = path.lastIndexOf("/");
        items.push(this.jumpItem(jumps, file, ["tr-menu-file"],
          `<span class="tr-menu-path">${escapeHtml(path.slice(slash + 1))}</span>` +
          (slash > 0 ? ` <span class="crash-menu-owner">${escapeHtml(path.slice(0, slash + 1))}</span>` : "")));
        for (const def of file.querySelectorAll(".tr-def[data-part]")) {
          // (As its heading says it, under its class, if it's in one.)
          const head = def.querySelector(":scope > .tr-def-head");
          const kind = head?.querySelector(".tr-def-kind")?.textContent ?? "";
          const name = head?.querySelector(".tr-def-name")?.textContent ?? "";
          const nested = def.parentElement.closest(".tr-def") && file.contains(def.parentElement.closest(".tr-def"));
          items.push(this.jumpItem(jumps, def, nested ? ["indent", "tr-menu-deeper"] : ["indent"],
            `<span class="crash-menu-owner">${escapeHtml(kind)}</span> ` +
            `<span class="tr-menu-def">${escapeHtml(name)}</span>`));
        }
      }
      if (section == here) {
        focus = items[0];
      }
      return {
        label: section.querySelector(".tr-section-label")?.textContent ?? "",
        detail: section == here ? "(this section)" : null,
        items,
      };
    });
    fillFittedMenu(jumps, groups, this.root.getBoundingClientRect().width - 16);
    return focus?.getFocusableElement();
  }

  jumpItem(jumps, element, classNames, html) {
    return new MenuItem({
      html: `<span class="tr-menu-mark">${this.icon(element.dataset.part)}</span> ${html}`,
      classNames,
      action: () => {
        jumps.hide();
        this.goTo(element);
      },
    });
  }

  showPatch(i) {
    const section = this.patchSections[i];
    if (!section) {
      return;
    }
    section.hidden = false;
    this.remember(section);
    section.scrollIntoView();
    // (Picked, even if the page can't scroll it under the breadcrumbs.)
    this.setCurrentPatch(i);
  }

  // (The patch whose section is under the breadcrumbs, or the first; or,
  // scrolled to the end, where the last ones can't get under them, the last
  // one whose top is in sight.)
  updateCurrentPatch() {
    if (!this.patchLink || this.patchSections.length == 0) {
      return;
    }
    const top = (document.querySelector(".breadcrumbs")?.getBoundingClientRect().bottom ?? 0) + 4;
    const scroller = document.scrollingElement;
    const atEnd = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2 &&
      scroller.scrollTop > 0;
    let current = 0;
    this.patchSections.forEach((section, i) => {
      const sectionTop = section.getBoundingClientRect().top;
      if (!section.hidden && (sectionTop <= top || (atEnd && sectionTop < window.innerHeight))) {
        current = i;
      }
    });
    this.setCurrentPatch(current);
  }

  setCurrentPatch(current) {
    if (current === this.currentPatch) {
      return;
    }
    this.currentPatch = current;
    const summary = TRY_REVIEW.patches[current]?.summary ?? "";
    this.patchLink.textContent = `Patch ${current + 1} of ${this.patchSections.length}`;
    this.patchLink.title = summary;
    this.patchLink.href = `#${this.patchSections[current].id}`;
  }

  // A small copy of the view's Then / Now / Then & Now choice in the
  // breadcrumbs, once the view's own scrolls out of sight.
  addThenNowCrumb() {
    const choice = this.root.querySelector(".tr-head .then-now-modes");
    const crumb = this.crumbs?.querySelector(".tr-crumb-then-now");
    if (!choice || !crumb || typeof IntersectionObserver != "function") {
      return;
    }
    // (Not a "then-now-modes", whose buttons then-now.js handles in their
    // view, which the breadcrumbs aren't in.)
    const copy = choice.cloneNode(true);
    copy.className = "tr-then-now-small";
    crumb.append(copy);
    copy.addEventListener("click", event => {
      const button = event.target.closest("button[data-mode]");
      if (!button) {
        return;
      }
      this.root.dataset.thenNowChosen = "";
      showThenNow(this.root, button.dataset.mode);
    });
    // (Its buttons pressed as the view's are.)
    const sync = () => {
      for (const button of copy.querySelectorAll("button[data-mode]")) {
        button.setAttribute("aria-pressed", String(button.dataset.mode == this.root.dataset.thenNow));
      }
    };
    new MutationObserver(sync).observe(this.root, { attributes: true, attributeFilter: ["data-then-now"] });
    sync();
    new IntersectionObserver(([entry]) => {
      crumb.hidden = entry.isIntersecting;
    }).observe(choice);
  }

  // The groups' dataflow diagrams (see the tools' `group_slicings`): each
  // group's changes' units of the push's slicing, and their neighbors in
  // other groups (dimmed), the ones sharing values or calls linked, laid out
  // by graph-records.js's `drawRecordGraph` (a row's units of the same class
  // in a record of them), each with its part's mark (kept up to date), going
  // to its part when clicked (its source, if it's ctrl-clicked); and with
  // their code, once asked to.
  drawSlicings() {
    const slicing = TRY_REVIEW.slicing;
    this.sliceMarks = [];
    if (!slicing?.groups || typeof drawRecordGraph == "undefined") {
      return;
    }
    for (const figure of this.root.querySelectorAll(".tr-slicing[data-group]")) {
      const data = slicing.groups[Number(figure.dataset.group)];
      if (!data?.nodes.length) {
        continue;
      }
      const canvas = figure.querySelector(".tr-slicing-canvas");
      const draw = withCode => {
        canvas.replaceChildren();
        this.sliceMarks = this.sliceMarks.filter(({ element }) => element.isConnected);
        drawRecordGraph(canvas, data.nodes, data.edges, {
          makeNode: node => this.sliceNode(slicing, node, withCode),
          // (Records' cells are too small for their code.)
          scopeOf: node => (withCode ? null : node.scope || null),
          edgeTitle: edge => edge.why.join("; "),
        });
      };
      draw(false);
      const button = figure.querySelector(".tr-slicing-code");
      button?.addEventListener("click", () => {
        const pressed = button.getAttribute("aria-pressed") != "true";
        button.setAttribute("aria-pressed", String(pressed));
        button.textContent = pressed ? "Hide their code" : "Show their code";
        draw(pressed);
      });
    }
    this.review.subscribe(() => {
      for (const { node, mark } of this.sliceMarks) {
        mark.replaceChildren(ReviewState.icon(this.review.effective(node.part).state));
      }
    });
  }

  sliceNode(slicing, node, withCode) {
    const element = document.createElement("a");
    element.className = `crash-graph-node tr-slice-node tr-slice-${node.change}` +
      (node.other != null ? " tr-slice-other" : "");
    // (Its source: the head's, or, all removed, the base's.)
    const removed = node.change == "removed";
    const [base, head] = slicing.query.replace(/^slice-rev:/, "").split("..");
    element.href = `/${TRY_REVIEW.tree}/rev/${removed ? base : head}/` +
      node.path.split("/").map(encodeURIComponent).join("/") +
      (node.lines.length ? `#${node.lines[0]}` : "");
    const file = node.path.split("/").pop();
    const [scope, text] = node.label == node.path ? ["", file] : graphNameLines(node.label);
    if (scope) {
      const line = document.createElement("div");
      line.className = "crash-graph-node-scope";
      line.textContent = scope;
      element.append(line);
    }
    const line = document.createElement("div");
    line.className = "crash-graph-node-text";
    if (node.part) {
      const mark = document.createElement("span");
      mark.className = "tr-slice-mark";
      mark.append(ReviewState.icon(this.review.effective(node.part).state));
      line.append(mark, " ");
      this.sliceMarks.push({ node, mark, element });
    }
    line.append(text);
    element.append(line);
    const lines = node.lines.length > 1
      ? `lines ${node.lines[0]}\u2013${node.lines[node.lines.length - 1]}`
      : node.lines.length ? `line ${node.lines[0]}` : "";
    const where = [
      node.label == node.path ? lines : `${file}${lines ? `, ${lines}` : ""}`,
      node.patch != null ? `patch ${node.patch + 1}` : null,
      node.change == "changed" ? null : node.change,
      node.other != null ? `group ${node.other + 1}` : null,
    ].filter(Boolean).join("; ");
    const sub = document.createElement("div");
    sub.className = "tr-slice-sub";
    sub.textContent = where;
    element.append(sub);
    // (What it removes, up to 8 lines, then its card's lines, what it adds
    // and the lines on their dataflow, by their numbers, with gaps; or, if
    // it has none, what it adds.)
    if (withCode && (node.text.length || node.old_text.length)) {
      const code = document.createElement("pre");
      code.className = "tr-slice-code";
      const add = (text, className, title) => {
        const line = document.createElement("div");
        line.className = className;
        line.textContent = text || " ";
        if (title) {
          line.title = title;
        }
        code.append(line);
      };
      const some = (text, className) => {
        for (const each of text.slice(0, 8)) {
          add(each, className);
        }
        if (text.length > 8) {
          add("\u2026", className);
        }
      };
      some(node.old_text, "tr-removed");
      if (node.card?.length) {
        let last = null;
        for (const line of node.card) {
          if (last !== null && line.line > last + 1) {
            add("\u22ef", "tr-slice-gap");
          }
          last = line.line;
          add(`${String(line.line).padStart(5)}  ${line.text}`,
              line.added ? "tr-added" : "tr-slice-flow", line.why?.join("\n"));
        }
      } else {
        some(node.text, "tr-added");
      }
      element.append(code);
    }
    element.title = `${node.label}\n${node.path}${lines ? `, ${lines}` : ""}` +
      (removed ? " (removed)" : "");
    element.addEventListener("click", event => {
      if (node.part && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
        event.preventDefault();
        this.go(node.part);
      }
    });
    return element;
  }

  // Going somewhere in the review (a picker's, the summary's or a diagram's
  // part, or a patch) is a history entry, so that Back returns to where
  // the page was (the browser keeps each entry's scroll position), and the
  // URL names the part (`#` its element's id), so that Enter in the URL bar
  // scrolls back to it, without reloading.
  remember(element) {
    const target = element.closest("[id]");
    if (target && location.hash != `#${target.id}`) {
      history.pushState(history.state, "", `#${target.id}`);
    }
  }

  // The lines a later patch of the push changes (see the tools'
  // `add_later_changes`), which are marked beside their blame strips, and
  // those their patch moved there (see the tools' `add_moved_lines`), marked
  // between their numbers and their code: hovering over one shows, above it,
  // where it moved from (the lines it was, in their hunk), and that later
  // patch's change (the lines its hunk adds); clicking one goes to its hunk
  // (a history entry, so that Back returns to the line).
  addLinePopups() {
    const changes = TRY_REVIEW.later || [];
    const moves = TRY_REVIEW.moved || [];
    if (!changes.length && !moves.length) {
      return;
    }
    const popup = document.createElement("div");
    popup.className = "tr-later-popup";
    popup.hidden = true;
    popup.setAttribute("role", "tooltip");
    document.body.append(popup);
    let shownFor = null;
    let hideTimer = null;
    const hide = () => {
      popup.hidden = true;
      shownFor = null;
    };
    // (A section of the popup: its heading, and its lines' code, a link to
    // the hunk whose element is `element`.)
    const section = (headText, lines, empty, element, title) => {
      const head = document.createElement("div");
      head.className = "tr-later-head";
      head.textContent = headText;
      const code = document.createElement("a");
      code.className = "tr-later-code";
      code.href = `#${element}`;
      code.title = title;
      for (const html of lines.length ? lines : [""]) {
        const line = document.createElement("code");
        line.className = "source-line";
        if (!html) {
          line.textContent = empty;
        } else {
          // (The tools' HTML, of the code's highlighting, escaped there.)
          TrustedHTML.set(line, html);
        }
        code.append(line);
      }
      code.addEventListener("click", event => {
        event.preventDefault();
        hide();
        const target = document.getElementById(element);
        if (target) {
          this.goTo(target);
        }
      });
      return [head, code];
    };
    const show = row => {
      clearTimeout(hideTimer);
      if (shownFor == row) {
        return;
      }
      const parts = [];
      const move = row.dataset.moved !== undefined ? moves[Number(row.dataset.moved)] : null;
      if (move) {
        const lines = move.first == move.last ? `line ${move.first}` : `lines ${move.first}\u2013${move.last}`;
        const where = move.def ? ` (in ${move.def})` : "";
        parts.push(...section(`Moved here from ${move.path} ${lines}${where}:`, move.lines, "",
                              move.element, "Go to where it moved from"));
      }
      const change = row.dataset.later !== undefined ? changes[Number(row.dataset.later)] : null;
      if (change) {
        parts.push(...section(`Patch ${change.patch + 1} changes this line to:`, change.lines,
                              "(it removes it)", change.element,
                              `Go to patch ${change.patch + 1}'s change`));
      }
      if (!parts.length) {
        return;
      }
      shownFor = row;
      popup.replaceChildren(...parts);
      popup.hidden = false;
      const rect = row.getBoundingClientRect();
      popup.style.left = `${Math.max(4, rect.left + window.scrollX + 24)}px`;
      popup.style.top = `${rect.top + window.scrollY - popup.offsetHeight - 2}px`;
    };
    const ROWS = ".tr-line.tr-later, .tr-line.tr-moved";
    this.root.addEventListener("mouseover", event => {
      const row = event.target.closest(ROWS);
      if (row) {
        show(row);
      }
    });
    const leave = () => {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(hide, 300);
    };
    this.root.addEventListener("mouseout", event => {
      if (event.target.closest(ROWS)) {
        leave();
      }
    });
    popup.addEventListener("mouseenter", () => clearTimeout(hideTimer));
    popup.addEventListener("mouseleave", leave);
  }

  // A line's rows (its hunk's, and its others on the page, ex: a moved
  // line's where it moved from; by their `data-line` keys, see the tools'
  // `side_html`) are highlighted together while one of them is hovered over,
  // for following a line between them.
  addLineHover() {
    let index = null;
    let shown = [];
    const rowsOf = row => {
      const key = row.dataset.line;
      // (Indexed at the first hover, and again for a row added since.)
      if (!index?.get(key)?.includes(row)) {
        index = new Map();
        for (const other of this.root.querySelectorAll(".tr-line[data-line]")) {
          const rows = index.get(other.dataset.line);
          if (rows) {
            rows.push(other);
          } else {
            index.set(other.dataset.line, [other]);
          }
        }
      }
      return index.get(key) || [row];
    };
    const clear = () => {
      for (const row of shown) {
        row.classList.remove("tr-hover");
      }
      shown = [];
    };
    this.root.addEventListener("mouseover", event => {
      const row = event.target.closest?.(".tr-line[data-line]");
      if (!row) {
        clear();
        return;
      }
      if (shown.includes(row)) {
        return;
      }
      clear();
      shown = rowsOf(row);
      for (const other of shown) {
        other.classList.add("tr-hover");
      }
    });
    this.root.addEventListener("mouseleave", clear);
  }

  // (Marking a part as being looked at is a history entry too, of its
  // element.)
  rememberLooking() {
    this.root.addEventListener("click", event => {
      const button = event.target.closest('.review-mark[data-state="looking"]');
      const box = button?.closest(".review-strip[data-part]");
      if (!box || this.review.own(box.dataset.part) == "looking") {
        return;
      }
      // (The part's element, not the strip's popup's.)
      const part = box.dataset.part;
      const element = [...this.root.querySelectorAll("[data-part][id]")]
        .find(each => each.dataset.part == part && each.contains(box)) ??
        box.closest("[id]");
      if (element) {
        this.remember(element);
      }
    }, true);
  }

  // The navigation panel's section of the review: its status (how many
  // parts are marked, that they're kept, or why not, and undo) and summary
  // (see `ReviewState.summary`), before the panel's "Debug" section.
  addPanelSection() {
    const content = document.getElementById("panel-content") || this.root.querySelector(".tr-head");
    const section = document.createElement("section");
    section.id = "tr-review-pane";
    const title = document.createElement("h4");
    title.textContent = "Your review";
    section.append(title);
    const box = document.createElement("p");
    box.className = "tr-status";
    this.statusText = document.createElement("span");
    this.statusText.className = "tr-status-text";
    this.undoButton = document.createElement("button");
    this.undoButton.type = "button";
    this.undoButton.className = "tr-undo";
    this.undoButton.textContent = "Undo";
    this.undoButton.title = "Undo your latest change to the marks or notes (Ctrl+Z)";
    this.undoButton.disabled = true;
    this.undoButton.addEventListener("click", () => this.review.undo());
    box.append(this.statusText, " ", this.undoButton);
    section.append(box);
    this.summaryBox = document.createElement("div");
    section.append(this.summaryBox);
    const debug = [...content.querySelectorAll(":scope > h4")].find(h4 => h4.textContent == "Debug");
    if (debug) {
      content.insertBefore(section, debug);
    } else {
      content.append(section);
    }
  }

  // Goes to a part (from the summary's lists): scrolls to it, and flashes
  // it, or says the facets hide it.
  go(part) {
    const element = [...this.root.querySelectorAll(
      ".tr-patch[data-part], .tr-file[data-part], .tr-def[data-part], .tr-hunk[data-part]")]
      .find(e => e.dataset.part == part);
    if (element) {
      this.goTo(element);
    }
  }

  goTo(element) {
    if (element.closest("[hidden]")) {
      this.statusText.textContent = "The navigation panel's facets hide it; clear them to see it.";
      return;
    }
    this.remember(element);
    element.scrollIntoView({ block: "start" });
    element.classList.remove("tr-flash");
    void element.offsetWidth;
    element.classList.add("tr-flash");
  }

  updateStatus() {
    const marks = Object.keys(this.review.data.marks).length;
    const notes = Object.keys(this.review.data.notes).length;
    const what = [
      `${marks} mark${marks == 1 ? "" : "s"}`,
      notes ? `${notes} note${notes == 1 ? "" : "s"}` : null,
    ].filter(Boolean).join(" and ");
    this.statusText.textContent = !this.review.persistent || this.review.error
      ? `${what}, not kept: this browser's storage (IndexedDB) isn't available here.`
      : this.review.pending > 0
        ? `${what}, saving\u2026`
        : `${what}, kept in this browser.`;
    this.undoButton.disabled = !this.review.canUndo();
  }

  // (A section, or patch, all of whose files the facets hide, is hidden.)
  hideEmpty() {
    for (const section of this.root.querySelectorAll(".tr-section")) {
      section.hidden = !section.querySelector(".tr-file:not([hidden])");
    }
    for (const patch of this.root.querySelectorAll(".tr-patch")) {
      const sections = patch.querySelectorAll(".tr-section");
      patch.hidden = sections.length > 0 && ![...sections].some(section => !section.hidden);
    }
  }
})();
