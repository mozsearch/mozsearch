/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A view of code then and now (see the tools' crash_stack.rs's
// `then_now_modes_html`, and bug_comment.rs), which loads this after its HTML.

// (Which code the view shows beside what it's of: then, now, or both side by
// side; see crash_stack.rs's `then_now_modes_html`.  At first, both if there's
// room for two columns of code beside what they're of, else then, again as the
// window's resized, until a button's pressed.)
function showThenNow(view, mode) {
  view.dataset.thenNow = mode;
  for (const button of view.querySelectorAll(".then-now-modes button[data-mode]")) {
    button.setAttribute("aria-pressed", String(button.dataset.mode == mode));
  }
}
function fitThenNow() {
  for (const view of document.querySelectorAll(".then-now-view:not([data-then-now-chosen])")) {
    const pair = view.querySelector(".then-now");
    const area = pair?.closest(".bug-comment-cards, .crash-stack");
    const line = pair?.querySelector(".source-line");
    if (!area || !line) {
      continue;
    }
    // (A column of code is about 100 of its characters, and the crash view's
    // frames' info is beside them.)
    const probe = document.createElement("span");
    probe.style.cssText = "position: absolute; visibility: hidden; white-space: pre";
    probe.style.font = getComputedStyle(line).font;
    probe.textContent = "0".repeat(100);
    document.body.append(probe);
    const column = probe.getBoundingClientRect().width;
    probe.remove();
    let room = area.clientWidth;
    const info = area.matches(".crash-stack") && area.querySelector(".crash-frame-info");
    if (info) {
      room -= info.getBoundingClientRect().width + 24;
    }
    showThenNow(view, room >= 2 * column + 16 ? "both" : "then");
  }
}
fitThenNow();
window.addEventListener("resize", fitThenNow);
document.addEventListener("click", event => {
  const button = event.target.closest(".then-now-modes button[data-mode]");
  if (!button) {
    return;
  }
  const view = button.closest(".then-now-view");
  view.dataset.thenNowChosen = "";
  showThenNow(view, button.dataset.mode);
});
