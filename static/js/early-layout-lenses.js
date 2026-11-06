/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

// The navigation panel's lenses, as the settings say, as this loads right after
// them; see `EarlyLayout` in settings.js.
typeof EarlyLayout != "undefined" && EarlyLayout.lenses();
