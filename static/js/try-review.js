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
    const crumbs = this.root.querySelector(".tr-crumbs");
    const breadcrumbs = document.querySelector(".breadcrumbs");
    if (crumbs && breadcrumbs) {
      breadcrumbs.append(crumbs);
      crumbs.hidden = false;
    }
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
