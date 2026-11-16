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
    const icon = part => ReviewState.MARKS.find(m => m.state == this.review.effective(part).state)?.icon ?? "";
    const item = (i, honorary) => new MenuItem({
      html: `<span class="tr-menu-mark">${icon(patches[i].part)}</span> ` +
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
    // (About as many rows in each column, at most 3 columns.)
    const rows = sorted.reduce((n, group) => n + group.items.length + 1, 0);
    const count = Math.min(3, Math.max(1, Math.ceil(rows / 16)));
    const perColumn = Math.ceil(rows / count);
    const columns = [[]];
    let inColumn = 0;
    for (const group of sorted) {
      if (inColumn > 0 && inColumn + group.items.length + 1 > perColumn && columns.length < count) {
        columns.push([]);
        inColumn = 0;
      }
      columns[columns.length - 1].push(group);
      inColumn += group.items.length + 1;
    }
    fillCrashMenu(menu, columns);
    return menu.menu.querySelector(`a[data-patch="${this.currentPatch ?? 0}"]:not(.tr-menu-honorary)`);
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
    if (!element) {
      return;
    }
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
