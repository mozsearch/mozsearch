/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * Showing the internals of a dataflow trace's black boxes (see the tools'
 * dataflow/mapping.rs and config_defaults/dataflow-mapping.toml): the page's
 * query again, with `follow-internals:` naming them (and those it already
 * did), for subway maps' collapsed nodes (see subway.js) and the dataflow
 * graph's (whose links, made without the query, are
 * `#follow-internals:<groups>`).
 */
window.FollowInternals = {
  url(groups) {
    const params = new URLSearchParams(document.location.search);
    const shown = [];
    const query = (params.get("q") ?? "")
      .replace(/(^|\s)follow-internals:(\S+)/g, (_, space, value) => {
        shown.push(...value.split(","));
        return space;
      })
      .trim();
    const all = [...new Set([...shown, ...groups])];
    params.set("q", `${query} follow-internals:${all.includes("all") ? "all" : all.join(",")}`);
    return `${document.location.pathname}?${params}`;
  },
};

document.addEventListener("click", event => {
  const link = event.target.closest?.("a");
  const href = link?.getAttribute("href") ?? link?.getAttribute("xlink:href");
  if (!href?.startsWith("#follow-internals:")) {
    return;
  }
  event.preventDefault();
  document.location.href = FollowInternals.url(href.slice("#follow-internals:".length).split(","));
});
