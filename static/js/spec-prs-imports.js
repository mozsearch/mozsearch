/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A spec repository's pull requests' page's imports' statuses (see\nspec_prs.liquid), until none wait or run.

(() => {
  // (Each pull request's import's status, until none wait or run.)
  const root = document.querySelector(".spec-prs-imports");
  const poll = async () => {
    let imports;
    try {
      imports = await (await fetch(root.dataset.statusUrl, { cache: "no-store" })).json();
    } catch (e) {
      setTimeout(poll, 5000);
      return;
    }
    for (const { number, state, text } of imports) {
      const badge = document.querySelector(`.spec-pr[data-path="${number}"] .spec-pr-import`);
      if (badge) {
        badge.dataset.state = state;
        badge.textContent = text;
      }
    }
    if (!imports.some(({ state }) => state == "queued" || state == "running")) {
      location.reload();
      return;
    }
    setTimeout(poll, 3000);
  };
  setTimeout(poll, 3000);
})();
