/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A crash set's form's status (see crash_set_form.liquid): while its crash\nreports are fetched, it says how far along that is, until it goes to the set.

(() => {
  const line = document.querySelector(".crash-set-status");
  const poll = async () => {
    let status;
    try {
      status = await (await fetch(line.dataset.statusUrl, { cache: "no-store" })).json();
    } catch (e) {
      setTimeout(poll, 3000);
      return;
    }
    line.dataset.state = status.state;
    if (status.state == "queued") {
      line.textContent = status.ahead > 0
        ? `Waiting to be fetched, after ${status.ahead} other${status.ahead == 1 ? "" : "s"}.`
        : "Waiting to be fetched.";
    } else if (status.state == "running") {
      line.textContent = `Fetching them: ${status.fetched} of ${status.total}.`;
    } else if (status.state == "done") {
      location.replace(status.url);
      return;
    } else {
      location.reload();
      return;
    }
    setTimeout(poll, 1000);
  };
  poll();
})();
