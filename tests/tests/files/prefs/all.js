/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// Default prefs, like modules/libpref/init/all.js (which the build
// preprocesses).

pref("test.js.pref", true);
#ifdef XP_WIN
  pref("test.js.platform", "windows");
#else
  pref("test.js.platform", "other");
#endif
sticky_pref("test.js.sticky", @MOZ_TEST_VALUE@);
lockPref("test.js.locked", 1);
