/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A spec repository's pull requests' page's import button (see spec_prs.liquid).

(() => {
  // (The button imports those of them the facets show which need it, if
  // they're few enough, saying how many.)
  const form = document.querySelector(".spec-prs-import-form");
  const button = form.querySelector("button");
  const limit = document.querySelector(".spec-prs-import-limit");
  const max = Number(form.dataset.maxBatch);
  const shown = () => [...document.querySelectorAll('.spec-pr[data-needed="true"]')].filter(pr => !pr.hidden);
  const update = () => {
    const count = shown().length;
    button.hidden = !count || count > max;
    limit.hidden = count <= max;
    button.textContent = count == 1 ? "Import this pull request" : `Import these ${count} pull requests`;
  };
  form.addEventListener("submit", () => {
    const url = new URL(form.action, location.href);
    url.searchParams.set("numbers", shown().map(pr => pr.dataset.path).join(","));
    form.action = url.toString();
  });
  new MutationObserver(update).observe(document.querySelector(".spec-pr-list"), {
    attributes: true,
    attributeFilter: ["hidden"],
    subtree: true,
  });
  update();
})();
