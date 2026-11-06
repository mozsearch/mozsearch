/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// The 404 page's "Go back history" link (rather than an inline handler, which
// our Content-Security-Policy doesn't allow).
for (const link of document.querySelectorAll("a.history-back")) {
  link.addEventListener("click", event => {
    event.preventDefault();
    history.back();
  });
}
