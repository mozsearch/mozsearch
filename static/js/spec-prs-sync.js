/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A spec repository's pull requests' page's sync's status (see spec_prs.liquid):\nwhile they're being synced, it says how far along that is, until the page\nreloads.

(() => {
  const root = document.querySelector(".spec-prs");
  const line = root.querySelector(".spec-prs-status");
  const poll = async () => {
    let status;
    try {
      status = await (await fetch(root.dataset.statusUrl, { cache: "no-store" })).json();
    } catch (e) {
      setTimeout(poll, 5000);
      return;
    }
    if (status.state == "queued") {
      line.textContent = "Waiting to sync.";
    } else if (status.state == "running") {
      line.textContent = `Syncing (${status.seconds}s so far): `;
      const step = document.createElement("code");
      step.textContent = status.step;
      line.append(step);
    } else {
      location.reload();
      return;
    }
    setTimeout(poll, 2000);
  };
  setTimeout(poll, 2000);
})();
