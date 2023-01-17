/**
 * Compactly defined setting names and defaults; expanded and interpreted by the
 * `Settings` class as its construction time in conjunction with
 * `SETTINGS_VERSION`.
 *
 * Although we define a more complex conceptual model for how settings work in
 * `settings.liquid`, we have a simple nested model here that operates based
 * on convention / specific names and it's on the `settings.liquid` page to make
 * sure it names its input "id" attributes to match with the automatic mangling
 * we perform below.
 *
 * General definition rules:
 * - The top level defines settings groups, and these should be named using
 *   camelCase.
 * - Inside each of those group definitions, each camelCase key names a setting
 *   that will be exposed as `Settings.[groupName].[keyName]` on the Settings
 *   object.  For form binding purposes, the names will be normalized from
 *   camelCase "fooBarBaz" to dash-delimited "foo-bar-baz", and the group name
 *   will be separated from the key name by a double-dash, like
 *   "group-name--key-name".
 * - A key name of "enabled" is treated as a feature gate.  The value exposed to
 *   JS will be a simple boolean (ex: `Settings.fancyBar.enabled`), but the
 *   storage value will be one of "" (to use the default feature gate), "alpha",
 *   "beta", or "release".
 * - Setting keys will have a dictionary that may contain the following keys:
 *   - `default`: This identifies the default value and also serves to indicate
 *     the type of the value.  This is not relevant for "enabled" keys.
 *   - `quality`: For the case of the special "enabled" featured gates, this
 *     expresses the current quality of the feature for feature gate logic
 *     purposes.  Legal values are "alpha", "beta", and "release".
 *   - `dependencies`: For the case of the special "enabled" feature gates, this
 *     expresses the other features that must also be enabled for this feature
 *     to be enabled.  Only the camelCase groupName needs to be identified.  The
 *     dependency check mechanism will run in as `SETTING_DEFS` is processed in
 *     its natural order, so features that depend on other features should
 *     appear after them in the dictionaries and will therefore emergently
 *     support transitive dependency checks correctly.
 *   - `introducedIn`: If present, indicates the `SETTINGS_VERSION` number
 *     version in which the setting was introduced.  The intent is to enable the
 *     ability to badge the settings button to let users know there are new
 *     settings that they might want to check out and then to help highlight
 *     and/or provide a table of contents with direct links to point out the
 *     new settings on the settings page without requiring the user to manually
 *     skim the page and guess what might be new.  This is used in conjunction
 *     with the `userSawVersion` and `userAckedVersion` storage values.
 */
const SETTING_DEFS = {
  global: {
    defaultFeatureGate: {
      default: "release",
    },
  },
  pageTitle: {
    lineSelection: {
      default: true,
    },

    stickySymbol: {
      default: true,
    },
  },
  contextMenu: {
    // There are a number of resources that may be gated behind Mozilla LDAP
    // access.  For discoverability reasons, we want to default this to true,
    // but for users without access, it should be possible to set this to false
    // so they don't have to see options they can't use.
    haveMozillaLdap: {
      default: true,
    }
  },
  fancyBar: {
    enabled: {
      quality: "alpha",
    }
  },
  diagramming: {
    enabled: {
      quality: "alpha",
    }
  },
};

const QUALITY_ORDERING = [
  "alpha",
  "beta",
  "release",
];

/**
 * Checks if the user's selected quality gate (falling back to their default)
 * is met/exceeded by the feature's current quality.
 */
function meetsQualityGate(userSpecific, userDefault, featureQuality) {
  let userCriteria = userSpecific || userDefault;
  let userIndex = QUALITY_ORDERING.indexOf(userCriteria);
  let featureIndex = QUALITY_ORDERING.indexOf(featureQuality);
  return featureIndex >= userIndex;
}

/**
 * This is just a specialized version of SETTING_DEFS where each top-level
 * group is conceptually associated with a widget which must usually must be
 * explicitly enabled by the user, but we do allow for a default which will be
 * populated on first load.  Currently there is no plan to build a widget
 * abstraction layer; the term exists to distinguish optional, loosely-coupled
 * features (widgets) from core features that will eventually be non-optional.
 * (Noting that there are still discussions to be had in this space, but the
 * intent is to set expectations appropriately.)
 *
 * The main differences from SETTINGS_DEFS above:
 * - `enabled` can be omitted; if it's omitted, a value of `{ default: false }`
 *   is assumed.  This is notably different from how feature `enabled` keys
 *   work.  You can express `dependencies` if you want, but in general it will
 *   probably be assumed that most widgets will depend on the "fancyBar" being
 *   enabled.
 */
const WIDGET_DEFS = {
  // Proposed in https://bugzilla.mozilla.org/show_bug.cgi?id=1808415 and with
  // its setting definitions stubbed here as an exercise for seeing how
  // widgets could be hooked up.  The widget doesn't actually exist yet.
  openInEditor: {
    enabled: {
      default: false,
    },
    linkTemplate: {
      default: "editor://open/?file={{tree}}/{{path}}",
    }
  }
}

/**
 * This version is stored in local storage as part of our persisted settings
 * structure.  In particular, we store it in the top level twice:
 * - `version`: The version of SETTING_DEFS that was last used to populate the
 *   `settings` field that contains the actual persisted settings values.
 * - `userSawVersion`: The `SETTINGS_VERSION` of the last time the user saw the
 *   settings page.  This would be used for quieting any badging that's
 *   notifying the user there are new settings.
 * - `userAckedVersion`: The `SETTINGS_VERSION` of the last time the user
 *   implicitly or explicitly confirmed they understand what the new settings
 *   are.  This differs from `userSawVersion` in that we might still highlight
 *   (and list in a TOC) new settings introduced between `userAckedVersion` and
 *   `version` even if we aren't displaying a badge.  In particular, it's our
 *   expectation that many users may find the (probably opt-in) badge
 *   distracting and that they likely would want to actually process the new
 *   settings later on.
 */
const SETTINGS_VERSION = 2;

/**
 * Convert a "camelCaseString" to "camel-case-string".
 */
function camelCaseToLowerCaseDash(ccStr) {
  return ccStr.replaceAll(/[A-Z]/g, x => `-${x.toLowerCase()}`);
}

/**
 * Convert a "group-name--key-name" string to a tuple of ["groupName", "keyName"].
 */
function convertDashedIdentifierToGroupAndName(idStr) {
  const pieces = idStr.split("--");
  return pieces.map(s => s.replace(/-[a-z]/g, x => x[1].toUpperCase()));
}

/**
 * Settings singleton which will block on loading LocalStorage data so that
 * setting data is available immediately after its initialization.  We can
 * potentially change this in the future.  Settings are exposed in a parallel
 * fashion to how they are defined in `SETTING_DEFS` and `WIDGET_DEFS`, so
 * the definition for `pageTitle.lineSelection` will be exposed at
 * `Settings.pageTitle.lineSelection`.
 *
 * Binding to forms on the `settings.html` page derived from the
 * `settings.liquid` template is handled via the `SettingsBinder` singleton
 * below.
 *
 * Storage is global for the entire origin and is not tree-specific at this
 * time.
 *
 * All settings storage happens in the single "settings" LocalStorage key,
 * stored as JSON and read only at page load time, currently.  In the future
 * we can have this listen for "storage" events to live-update, but the
 * complexity does not seem merited at this time.
 *
 * The choice of a single JSON key/value is made because:
 * - We want richer types and structures than just raw strings.
 * - There are advantages to the coherency of a single key/value, including
 *   self-descriptive versioning.
 * - We read all of the settings at page load time, and given this access
 *   pattern, and the fact that we don't/won't store much other data in
 *   LocalStorage, it's easier and faster for Gecko's LocalStorage (NG) if it's
 *   just giving us a single value.  (Actually, if we do start storing other
 *   data in LS, using a single key/value ends up even better.)
 *
 * The choice of LocalStorage is the most pragmatic choice for storing a small
 * amount of data in a way that can be used to impact UI to minimize flashes of
 * changed content.  Cookies are not appropriate because they would be sent to
 * the server (which we neither want nor need) and `document.cookie` is limited
 * to 7-days by anti-tracking policy.  Firefox's LocalStorage (NG) will preload
 * data when it knows the connection is going to happen, and will keep it around
 * in memory so that file I/O should be avoided for searchfox.  This differs
 * from options like IndexedDB and the Cache API.
 */
const Settings = new (class Settings {
  #canonicalData;

  constructor() {
    // This will synchronously block if the browser has not been able to preload
    // the LocalStorage for the origin.  This is a hard-block and will not
    // spin an event loop.  This is the worst part of LocalStorage.
    this.#canonicalData = this.#loadCanonicalData();
    this.#applyAndTransformCanonicalDataToSelf();
  }

  #mergeSettingsCanonicalData(settingsRoot, groupName, groupDefs, isWidget) {
    let groupVals = settingsRoot[groupName];
    if (!groupVals) {
      groupVals = settingsRoot[groupName] = {};
    }

    for (const [keyName, keyDef] of Object.entries(groupDefs)) {
      // There's currently no concept of migration, so we have nothing to do if
      // the key already exist in the values dictionary.
      if (keyName in groupVals) {
        continue;
      }

      if (keyName === "enabled") {
        // For widgets we propagate the default, but for features we use an
        // default
        if (isWidget) {
          groupVals[keyName] = ("default" in keyDef) ? keyDef.default : false;
        } else {
          groupVals[keyName] = "";
        }
      } else {
        groupVals[keyName] = keyDef.default;
      }
    }
  }

  #generateDefaultSettings(parseFailed) {
    const data = {
      version: SETTINGS_VERSION,
      // In order to help debug storage-related problems, we want to track when
      // the data structured was first created.  These values will never be
      // submitted anywhere, although we may ask the user to help
      created: {
        version: SETTINGS_VERSION,
        timestamp: Date.now(),
        // Did we have a local storage payload but we failed to parse it?
        parseFailed,
      },
      userSawVersion: SETTINGS_VERSION,
      userAckedVersion: SETTINGS_VERSION,
      settings: {},
    };

    // Currently, the upgrade logic just fills in values it doesn't already
    // know.
    return this.#upgradeSettings(data);
  }

  #upgradeSettings(data) {
    for (const [groupName, groupDefs] of Object.entries(SETTING_DEFS)) {
      this.#mergeSettingsCanonicalData(data.settings, groupName, groupDefs, false);
    }

    for (const [widgetName, widgetDefs] of Object.entries(WIDGET_DEFS)) {
      this.#mergeSettingsCanonicalData(data.settings, widgetName, widgetDefs, true);
    }
    return data;
  }

  /**
   * Load the settings data from LocalStorage, creating default values if there
   * was no existing data, and upgrading existing data if appropriate.  Data
   * will automatically be written to LocalStorage if any changes may have
   * happened.
   *
   * Note that the returned data has not had feature gates applied; those are
   * only interpreted
   */
  #loadCanonicalData() {
    const strData = window.localStorage.getItem("settings");
    let data = null;
    let parseFailed = false;
    if (strData !== null) {
      try {
        data = JSON.parse(strData);
      } catch(ex) {
        parseFailed = true;
      }
    }
    if (data === null) {
      data = this.#generateDefaultSettings(parseFailed);
      this.#saveCanonicalData(data);
    } else if (data.version < SETTINGS_VERSION) {
      data = this.#upgradeSettings(data);
      this.#saveCanonicalData(data);
    }
    return data;
  }

  #saveCanonicalData(explicitData) {
    if (explicitData) {
      this.#canonicalData = explicitData;
    }
    const strData = JSON.stringify(this.#canonicalData);
    window.localStorage.setItem("settings", strData);
  }

  /**
   * Applies the storage-representation data in `this.#canonicalData` to `this`,
   * applying feature-gating logic transformations in the process.  We freeze
   * the group value objects, replacing them each time this method is called.
   * No attempt is made to protect the group objects themselves because that
   * seems like a less likely problem.
   */
  #applyAndTransformCanonicalDataToSelf() {
    const settings = this.#canonicalData.settings;
    const userDefaultQualityGate = settings.global.defaultFeatureGate;

    for (const [groupName, groupValues] of Object.entries(settings)) {
      let transformed = {};
      const isWidget = groupName in WIDGET_DEFS;
      let groupDef;
      if (isWidget) {
        groupDef = WIDGET_DEFS[groupName];
      } else {
        groupDef = SETTING_DEFS[groupName];
      }

      for (const [keyName, keyValue] of Object.entries(groupValues)) {
        if (keyName === "enabled") {
          let enabled;
          if (isWidget) {
            enabled = keyValue;
          } else {
            enabled = meetsQualityGate(keyValue, userDefaultQualityGate, groupDef.enabled.quality);
          }
          let maybeDeps = groupDef[keyName]?.dependencies;
          if (maybeDeps) {
            for (const depGroupName of maybeDeps) {
              if (!this[depGroupName].enabled) {
                enabled = false;
              }
            }
          }
          transformed.enabled = enabled;
        } else {
          transformed[keyName] = keyValue;
        }
      }

      Object.freeze(transformed);
      this[groupName] = transformed;
    }
  }

  __hasUnseenSettings() {
    return this.#canonicalData.userSawVersion < SETTINGS_VERSION;
  }

  __markSettingsSeen() {
    this.#canonicalData.userSawVersion = SETTINGS_VERSION;
    this.#saveCanonicalData();
  }

  __hasUnacknowledgedSettings() {
    return this.#canonicalData.userAckedVersion < SETTINGS_VERSION;
  }

  __markSettingsAcknowledged() {
    this.#canonicalData.userAckedVersion = SETTINGS_VERSION;
    this.#saveCanonicalData();
  }

  __lookupSettingFromId(id) {
    const [groupName, keyName] = convertDashedIdentifierToGroupAndName(id);
    const isWidget = groupName in WIDGET_DEFS;
    let groupDef;
    if (isWidget) {
      groupDef = WIDGET_DEFS[groupName];
    } else {
      groupDef = SETTING_DEFS[groupName];
    }
    if (!groupDef) {
      return null;
    }
    const keyDef = groupDef[keyName];

    let type;
    let coerceFromString = x => x;
    if (keyName === "enable") {
      if (isWidget) {
        type = "feature-gate";
      } else {
        type = "boolean"
      }
    } else {
      type = typeof(keyDef);
    }

    if (type === "number") {
      coerceFromString = x => parseInt(x, 10)
    } else if (type === "boolean") {
      // assuming checkbox with default "on" value if checked.
      coerceFromString = x => x.length > 0;
    }
    // the identity transform is appropriate for "feature-gate" and everything
    // else right now.

    let rawValue = this.#canonicalData.settings[groupName][keyName];

    return {
      isWidget,
      groupName,
      groupDef,
      keyName,
      keyDef,
      type,
      rawValue,
      coerceFromString,
    };
  }

  __setValueFromIdSpace(id, value) {
    const { groupName, keyName } = this.__lookupSettingFromId(id);

    this.#canonicalData.settings[groupName][keyName] = value;
    this.#saveCanonicalData();
    this.#applyAndTransformCanonicalDataToSelf();
  }
})();

/**
 * Very basic routing / URL parsing / URL generating support.  This primarily
 * exists to try and support varying URLs based on user settings as it relates
 * to using the "search" endpoint versus the "query" endpoint and things like
 * that.  Searchfox's URL scheme is stable and so, apart from experimental
 * features that may not end up making it to "release", there is no reason to
 * favor using this class over inline URL generation.  (And in particular, if
 * you can generate a URL on the server as part of the HTML we serve, you
 * absolutely should.)
 */
const Router = new (class Router {
  constructor() {
    const pathParts = document.location.pathname.split("/");
    this.treeName = pathParts[1];
    this.endpoint = pathParts[2];
    if (this.endpoint === "pages") {
      this.page = pathParts.slice(3).join("/");
    } else if (this.endpoint === "rev") {
      this.rev = pathParts[3];
      this.sourcePath = pathParts.slice(4).join("/");
    } else if (this.endpoint === "source") {
      this.sourcePath = pathParts.slice(3).join("/");
    }
  }
})();

/**
 * Self-activating singleton which will automatically bind itself to the
 * `settings.html` page derived from the `settings.liquid` template.  In the
 * future we may also provide a means for adding interactive mechanisms to
 * opt-out of features without visiting the settings page, and that would also
 * want to operate through this class.
 */
const SettingsBinder = new (class SettingsBinder {
  constructor() {
    // If we are the settings page, bind form elements and do any template
    // expansions for the feature gate select payloads.  Our JS script is
    // currently loaded as part of the `scroll_footer.liquid` template which
    // comes after all content and so the DOM should therefore already exist.
    if (Router.page === "settings.html") {
      this.bindAndExpandTemplatedForms();
    }
  }

  bindAndExpandTemplatedForms() {
    // Make sure we only manipulate the content area and don't interfere with
    // the search UI.

    const root = document.querySelector("#content");

    // Let's add an idempotency guard that complains in order to help shine a
    // light on any logic problems which might otherwise result in weirdness.
    if ("boundSettings" in root) {
      console.error("Attempted to re-bind settings!");
      throw new Error("Redundant attempt to bind settings.");
    }
    root.boundSettings = true;

    const featureGateOptions = root.querySelector("#feature-gate-options");

    // Neutralize all form elements so nothing ever submits anywhere, as this is
    // all content-side.
    for (const form of root.querySelectorAll("form")) {
      form.addEventListener("submit", (evt) => { evt.preventDefault(); });
    }

    // Bind all form inputs
    for (const elem of root.querySelectorAll("input, select")) {
      const info = Settings.__lookupSettingFromId(elem.id);
      if (!info) {
        console.warn("Thought about binding to", elem, "with id", elem.id, "but could not.");
        continue;
      }
      if (elem.tagName === "INPUT") {
        if (elem.type === "checkbox") {
          elem.checked = Settings[info.groupName][info.keyName];
          elem.addEventListener("change", () => {
            Settings.__setValueFromIdSpace(elem.id, elem.checked);
          });
        } else if (elem.type === "text") {
          elem.value = Settings[info.groupName][info.keyName];
          elem.addEventListener("change", () => {
            Settings.__setValueFromIdSpace(elem.id, elem.value);
          });
        } else {
          console.warn("Don't know how to bind to", elem, "with type", elem.type);
        }
      } else if (elem.tagName === "SELECT") {
        // Enabling
        if (!info.isWidget && info.keyName === "enabled") {
          // To reduce maintenance if we change the feature gate payloads, we
          // just use our template clone on the inside, and it could also make
          // sense to may set/propagate any other attributes as appropriate.
          elem.appendChild(featureGateOptions.content.cloneNode(true));
        }
        elem.value = info.rawValue;
        elem.addEventListener("change", () => {
          Settings.__setValueFromIdSpace(elem.id, elem.value);
        });
      }
    }

    // Bind things that say what current qualities are:
    for (const elem of root.querySelectorAll('[id^="quality--"]')) {
      const useId = elem.id.substring("quality--".length);
      const info = Settings.__lookupSettingFromId(useId);
      elem.textContent = info.keyDef.quality;
    }
  }
})();
