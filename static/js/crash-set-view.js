/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A crash set's view (see the tools' crash_sets.rs's `render_crash_set`),
// which loads this after its HTML.
//
// Its families: as the navigation panel's facets filter its crashes' rows
// (facets.js's "facetschange"), its families and variants say how many of
// those shown they have, those without any hide, and their latest crashes'
// links go to the latest shown.
(() => {
  const body = document.querySelector(".crash-set-crashes .facet-items");
  if (!body) {
    return;
  }
  const rows = [...body.querySelectorAll(".crash-set-crash")].map(row => ({
    row,
    values: JSON.parse(row.dataset.facets).family || [],
    href: row.querySelector(".crash-set-crash-link")?.getAttribute("href"),
  }));
  const plural = n => `${n} ${n == 1 ? "crash" : "crashes"}`;
  body.addEventListener("facetschange", () => {
    const shown = rows.filter(entry => !entry.row.hidden);
    const filtered = shown.length < rows.length;
    const show = (element, value, count, total) => {
      const n = shown.filter(entry => entry.values.includes(value)).length;
      element.hidden = n == 0;
      count.textContent = filtered ? `${n} of ${plural(total)}` : plural(total);
      return n;
    };
    for (const family of document.querySelectorAll(".crash-family")) {
      const count = family.querySelector(".crash-family-count");
      count.dataset.total ??= rows.filter(entry => entry.values.includes(family.dataset.family)).length;
      show(family, family.dataset.family, count, Number(count.dataset.total));
      for (const variant of family.querySelectorAll(".crash-variant")) {
        const value = variant.dataset.variant;
        const variantCount = variant.querySelector(".crash-variant-count");
        variantCount.dataset.total ??= rows.filter(entry => entry.values.includes(value)).length;
        if (show(variant, value, variantCount, Number(variantCount.dataset.total))) {
          // (Rows are the latest first.)
          const latest = shown.find(entry => entry.values.includes(value));
          variant.querySelector(".crash-variant-latest")?.setAttribute("href", latest.href);
        }
      }
    }
  });
})();

// (The set's use, for the next index's carry-over; see the tools'
// `CrashSetStore::touch`.)
navigator.sendBeacon?.(document.currentScript.dataset.touchUrl);
