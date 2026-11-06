/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A paged spec's rendered view's links to anchors that aren't on the page (ex:
// to the first page, from outside, or from the page, which only changes its
// fragment, and doesn't reload it; see search.js's popstate handler) go to
// their pages, by its `/anchor/` URL (this script's `data-anchor-base`; see
// the tools' `format::format_rendered`).
(() => {
  const base = document.currentScript.dataset.anchorBase;
  const go = () => {
    const id = decodeURIComponent(location.hash.slice(1));
    if (id && !/^\d+$/.test(id) && !document.getElementById(id)) {
      location.replace(base + encodeURIComponent(id));
    }
  };
  go();
  addEventListener("hashchange", go);
})();
