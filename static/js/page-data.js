/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * The page's data, which it has as JSON data blocks (`<script
 * type="application/json" data-global="NAME">`, which aren't scripts, so our
 * Content-Security-Policy, which allows no inline scripts, allows them), as the
 * globals our scripts read (ex: `SYM_INFO`).  A block with `data-append` is
 * one of its global's array's items (ex: each graph's `GRAPH_EXTRA`), and one
 * with `data-merge` has properties of its global's object (ex: the query
 * page's `SYM_INFO`, beside a view's own symbols' information).  The globals
 * are all-caps names, and only those (so no block can set ex: `onload`).
 *
 * The blocks' JSON escapes "<", ">" and "&" (as `\u003c` and so on), so that
 * no string in them can end their element (see the tools' `json_data_block`).
 */
for (const block of document.querySelectorAll('script[type="application/json"][data-global]')) {
  const name = block.dataset.global;
  if (!/^[A-Z][A-Z0-9_]*$/.test(name)) {
    console.warn("Not a page global's JSON data block:", name);
    continue;
  }
  const value = JSON.parse(block.textContent);
  if ("append" in block.dataset) {
    if (!Array.isArray(window[name])) {
      window[name] = [];
    }
    window[name].push(value);
  } else if ("merge" in block.dataset) {
    window[name] = Object.assign(window[name] || {}, value);
  } else {
    window[name] = value;
  }
}
