/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * The only way our scripts insert HTML they build as strings.
 *
 * Our pages' Content-Security-Policy (see scripts/nginx-setup.py) requires
 * Trusted Types for HTML sinks, so assigning a string to `innerHTML` (or the
 * like) throws, but for this policy's, `searchfox` (which the policy's
 * `trusted-types` names).  The HTML must still be built with everything from
 * the page's data escaped (see `TrustedHTML.escape`); as a backstop, browsers
 * with the HTML Sanitizer API get it through `setHTML`, which keeps our
 * markup (its sanitizer allowing every element and attribute) but always
 * removes whatever could run script (`<script>`, event handler attributes,
 * `javascript:` URLs, ...).
 */
var TrustedHTML = new (class TrustedHTML {
  constructor() {
    this.policy = window.trustedTypes?.createPolicy("searchfox", {
      createHTML: html => html,
    });
  }

  /** Replace an element's contents with HTML. */
  set(element, html) {
    if (typeof element.setHTML === "function") {
      element.setHTML(html, { sanitizer: {} });
      return;
    }
    element.innerHTML = this.policy ? this.policy.createHTML(html) : html;
  }

  /**
   * A URL from the page's data (ex: a commit message's review link), if it's
   * an http(s) one (not ex: `javascript:`), else null.
   */
  httpURL(url) {
    try {
      const parsed = new URL(url, document.location.href);
      return ["http:", "https:"].includes(parsed.protocol) ? parsed.href : null;
    } catch (e) {
      return null;
    }
  }

  /** Escape text for HTML text or a quoted attribute value. */
  escape(text) {
    return String(text)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }
})();
