/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

const lazy = {};

XPCOMUtils.defineLazyPreferenceGetter(lazy, "platform", "test.js.platform", "");

export function readPrefs(branch) {
  const enabled = Services.prefs.getBoolPref("test.js.pref", false);
  Services.prefs.setIntPref("test.int", 3);
  Services.prefs.getDefaultBranch("").getBoolPref("test.bool");
  Services.prefs.addObserver("test.str", () => {});
  // (A relative name on another branch isn't a pref's.)
  branch.getBoolPref("enabled");
  return enabled && Preferences.get("test2.uint");
}

export async function setUp() {
  await SpecialPowers.pushPrefEnv({
    set: [
      ["test.js.pref", true],
      ["test.int", 5],
    ],
    clear: [["test.bool"]],
  });
  Preferences.addAll([{ id: "test.js.locked", type: "int" }]);
}
