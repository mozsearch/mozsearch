/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// The navigation panel, docked as the fancy bar (if the settings say so)
// before the page's contents come, as this loads right after it; see
// `EarlyLayout` in settings.js.
typeof EarlyLayout != "undefined" && EarlyLayout.panel();
