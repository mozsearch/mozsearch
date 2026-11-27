/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A try push's import's status (see try_import.liquid): while it's being
// imported, it says how far along that is, until the page reloads; once its
// history is (`analyzing`), it links to its review, which is there without
// its semantic analysis until that's done too.

(() => {
  const root = document.querySelector(".try-import");
  const line = root.querySelector(".try-import-status");
  const poll = async () => {
    let status;
    try {
      status = await (await fetch(root.dataset.statusUrl)).json();
    } catch (e) {
      setTimeout(poll, 5000);
      return;
    }
    if (status.state == "queued") {
      line.textContent = status.ahead > 0
        ? `Waiting to be imported, after ${status.ahead} other${status.ahead == 1 ? "" : "s"}.`
        : "Waiting to be imported.";
    } else if (status.state == "running") {
      line.textContent = `Importing (${status.seconds}s so far): `;
      const step = document.createElement("code");
      step.textContent = status.step;
      line.append(step);
    } else if (status.state == "analyzing") {
      // (Its history is imported, so its review is there, without its
      // semantic analysis, which is waiting or running; the page reloads
      // once that's done too.)
      const cost = status.tus ? ` of ${status.tus} translation unit${status.tus == 1 ? "" : "s"}` : "";
      const state = status.seconds !== undefined
        ? `is running (${status.seconds}s so far${status.step ? `, at ${status.step}` : ""})`
        : status.ahead > 0 ? `is waiting, after ${status.ahead} other${status.ahead == 1 ? "" : "s"}`
        : "is next";
      const link = document.createElement("a");
      link.href = location.pathname;
      link.textContent = "Its review is ready";
      line.replaceChildren(link, `, but its semantic analysis${cost} ${state}: its jumps, references and diagrams come once it's done.`);
    } else {
      location.reload();
      return;
    }
    setTimeout(poll, 2000);
  };
  setTimeout(poll, 2000);
})();
