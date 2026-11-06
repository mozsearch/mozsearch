/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// A rendered spec's pull requests' list (the `<details>` before this script;
// see rendered_prs.liquid) is open or closed as it was last (see
// spec-pr-view.js, which remembers it), as soon as it's parsed.
(() => {
  const box = document.currentScript.previousElementSibling;
  const open = localStorage.getItem(storageKey("spPrListOpen"));
  if (open !== null) {
    box.open = open == "true";
  }
})();
