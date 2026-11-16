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

// (A menu's groups in columns of about as many rows, at most 3.)
function menuColumns(groups) {
  const rows = groups.reduce((n, group) => n + group.items.length + 1, 0);
  const count = Math.min(3, Math.max(1, Math.ceil(rows / 16)));
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
    this.root.dataset.reviewReady = "";
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
    fillCrashMenu(menu, menuColumns(sorted));
    return menu.menu.querySelector(`a[data-patch="${this.currentPatch ?? 0}"]:not(.tr-menu-honorary)`);
  }

  // (A part's mark's icon: its own, or what it has from its container.)
  icon(part) {
    return ReviewState.MARKS.find(m => m.state == this.review.effective(part).state)?.icon ?? "";
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
      // (Within the view, which the navigation panel can cover the right of.)
      const over = menu.getBoundingClientRect().right - this.root.getBoundingClientRect().right;
      if (over > 0) {
        menu.style.left = Math.max(0, menu.offsetLeft - over) + "px";
      }
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
    fillCrashMenu(jumps, menuColumns(groups));
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
    section.scrollIntoView();
    history.replaceState(history.state, "", `#${section.id}`);
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
        mark.textContent = this.icon(node.part);
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
      mark.textContent = this.icon(node.part);
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
