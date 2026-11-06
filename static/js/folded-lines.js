/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// (Excerpts' folded lines show on request, ex: a crash view's frames', a bug
// comment's view's cards', and a lock-held view's cards'; see the tools'
// crash_stack.rs and lock_held.rs.)
document.addEventListener("click", event => {
  const unfold = event.target.closest(".crash-unfold, .lock-held-unfold");
  if (!unfold) {
    return;
  }
  unfold.previousElementSibling.hidden = false;
  unfold.remove();
});
