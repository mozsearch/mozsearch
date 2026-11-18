/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

"use strict";

/**
 * Results as one types in query boxes (asuth, 2026-11-18; see the notes'
 * query-as-you-type-plan.md): in trees searching with queries, and on
 * queries' pages, a pause in typing a query worth searching as it's typed
 * (see `QueryGate`) loads its page, the one Enter would, in a frame over the
 * page's content (see `QueryResultsFrames`), rather than replacing the page.
 * Its scripts run in the frame as on any page, with their own globals, which
 * go with the frame.
 */

/**
 * Which queries are searched as they're typed, and how soon, by the query
 * language's terms (the pipeline-server's `/{tree}/query-terms`; see the
 * tools' `chew_query::terms_table`).
 */
class QueryGate {
  constructor(table) {
    this.terms = table.terms || {};
    // (Input a DWIM rule matches may go elsewhere: at once, by `/search`, ex:
    // a try push's page, which imports it; or by its query's page, if the
    // search finds nothing.  A pattern this can't parse stops all searching
    // as one types, rather than risk that.)
    this.dwim = [];
    this.broken = false;
    for (const source of table.dwim || []) {
      try {
        this.dwim.push(new RegExp(source));
      } catch (e) {
        console.error("query-frames: a DWIM pattern isn't JavaScript's:", source, e);
        this.broken = true;
      }
    }
  }

  /**
   * The terms of `raw`, as the tools' query parser (the `query-parser`
   * crate) finds them: separated by ASCII whitespace; `-` negating one; a
   * quoted one (`'...'` or `"..."`, with backslashes escaping quotes) a value
   * without a key; else a key, if there's a `:`, and its value (quoted, or up
   * to the next whitespace).  `open`: its quote isn't closed.
   */
  static tokenize(raw) {
    const terms = [];
    const space = c => c == " " || c == "\t" || c == "\n" || c == "\r" || c == "\f";
    const n = raw.length;
    let i = 0;
    const quoted = () => {
      const quote = raw[i++];
      let value = "";
      while (i < n) {
        const c = raw[i];
        if (c == "\\" && i + 1 < n) {
          const next = raw[i + 1];
          value += next == "'" || next == '"' || next == "\\" ? next : `\\${next}`;
          i += 2;
        } else if (c == quote) {
          i++;
          return { value, open: false };
        } else {
          value += c;
          i++;
        }
      }
      return { value, open: true };
    };
    while (i < n) {
      if (space(raw[i])) {
        i++;
        continue;
      }
      let negated = false;
      if (raw[i] == "-") {
        i++;
        if (i >= n || space(raw[i])) {
          terms.push({ negated: false, key: null, value: "-", open: false });
          continue;
        }
        negated = true;
      }
      if (raw[i] == "'" || raw[i] == '"') {
        terms.push({ negated, key: null, ...quoted() });
        continue;
      }
      let token = "";
      while (i < n && !space(raw[i]) && raw[i] != ":") {
        token += raw[i++];
      }
      if (raw[i] != ":") {
        terms.push({ negated, key: null, value: token, open: false });
        continue;
      }
      i++;
      if (raw[i] == "'" || raw[i] == '"') {
        terms.push({ negated, key: token, ...quoted() });
        continue;
      }
      let value = "";
      while (i < n && !space(raw[i])) {
        value += raw[i++];
      }
      terms.push({ negated, key: token, value, open: false });
    }
    return terms;
  }

  /**
   * Whether `input` (what's typed) is searched as it's typed, and if it is,
   * how much text it searches for (see `weight`).  As the tools'
   * `chew_query` takes them: a word that isn't a term's (ex: a URL's scheme,
   * or `Foo` in an unquoted `Foo::Bar`) is text, and a negated word is text
   * (but `-path:` and `-pathre:`).  It isn't searched:
   * - if a DWIM rule matches it (see the constructor);
   * - while a quote is open;
   * - if a term isn't incremental (ex: a diagram's, whose identifier must be
   *   exact; see the tools' `TermConfig::incremental`), or its value is empty
   *   or not one of those it can have (ex: `case:sen`);
   * - with less than 3 characters of text (see `weight`).
   */
  check(input) {
    // (Without the helper prefix, ". "; see the tools'
    // `dwim::without_helper_prefix`.)
    const text = input.replace(/^\s*\.\s+/, "").trim();
    if (!text || this.broken || this.dwim.some(pattern => pattern.test(text))) {
      return { searchable: false, weight: 0 };
    }
    let searched = "";
    for (const term of QueryGate.tokenize(text)) {
      // (A lone `-` is a negated term's start, ex: `-path:`'s.)
      if (term.open || (term.key === null && !term.negated && term.value == "-")) {
        return { searchable: false, weight: 0 };
      }
      const qualified = term.value.startsWith(":") && !term.value.startsWith("::");
      const info = term.key !== null && !qualified ? this.terms[term.key] : undefined;
      if (term.negated) {
        if (term.key == "path" || term.key == "pathre") {
          if (!info || !term.value) {
            return { searchable: false, weight: 0 };
          }
          continue;
        }
        if (info) {
          return { searchable: false, weight: 0 };
        }
        searched += term.value;
        continue;
      }
      if (!info || term.key == "default") {
        searched += term.key !== null && term.key != "default" ? `${term.key}:${term.value}` : term.value;
        continue;
      }
      if (!info.incremental || !term.value || (info.values && !info.values.includes(term.value))) {
        return { searchable: false, weight: 0 };
      }
      if (term.key == "text" || term.key == "file" || term.key == "idprefix") {
        searched += term.value;
      }
    }
    const weight = QueryGate.weight(searched);
    return { searchable: weight >= 3, weight };
  }

  /**
   * How much `text` says, for how soon it's searched: its characters, but
   * CJK characters (Han, kana and Hangul), each of which says as much as a
   * few others, as two.
   */
  static weight(text) {
    let weight = 0;
    for (const c of text) {
      weight += QueryGate.CJK.test(c) ? 2 : 1;
    }
    return weight;
  }

  static CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

  /**
   * How long after the last keystroke a query of `weight` is searched for
   * (asuth: "a query for only 3 characters is potentially quite expensive and
   * also quite unlikely to be useful ... we might start that higher at 500ms
   * when we're only at 3 characters and only transition down to 300ms as we
   * get to 6 characters"): 500 ms at 3, down to 300 ms at 6 and more.
   */
  static delay(weight) {
    return Math.round(500 - 200 * Math.min(Math.max(weight - 3, 0), 3) / 3);
  }
}
// (A global class isn't the window's property, which tests use.)
window.QueryGate = QueryGate;

/**
 * A query box's results as one types (see the file's comment), in frames over
 * the page's content, below its header.
 *
 * - **Loading**: one frame loads at a time; a query typed meanwhile is loaded
 *   once it's done (its results are shown, as they say something, and the
 *   server isn't asked for more than one at a time).  A frame loads hidden and
 *   is shown once its page is parsed (it says so: `queryframeready`), and the
 *   one shown before goes.  A page that doesn't say (ex: an error's plain
 *   text) is shown as it is, and nothing more is searched as it's typed until
 *   Enter.
 * - **History** (asuth's): typing is a session, whose first results push an
 *   entry (their page's URL), which later ones replace.  Enter makes the
 *   session's results (with any facets chosen in them) the page, in its entry;
 *   Escape or Back, during the session or after Enter, goes back to what was
 *   there before; typing after Enter is a new session, whose Escape goes back
 *   to the page Enter made.
 * - **Elsewhere**: links in the frame go to the whole window (see the frames'
 *   part below), as does anything else the frame's page goes to.
 */
class QueryResultsFrames {
  // (Whether the page was made by a query box's Enter, for its Escape; see
  // `onSubmit`.)
  static ENTERED_KEY = "query-frames-entered";

  constructor(input) {
    this.input = input;
    this.form = input.form;
    this.searchBox = document.getElementById("search-box");
    this.tree = document.getElementById("data")?.dataset.tree;
    // (What the box has on the page, which a session's Escape restores.)
    this.pageValue = input.value;
    this.gate = null;
    this.gateLoading = false;
    this.timer = null;
    // The frame loading (at most one), and the one shown: each `{ frame,
    // query, loads, ready }`.
    this.loading = null;
    this.shown = null;
    // Whether a query was typed while one loaded.
    this.more = false;
    // The typing session (see the class's comment), and the one whose Back
    // `endSession` started.
    this.session = null;
    this.popping = null;
    // Stopped by a failed load, until Enter.
    this.stopped = false;

    input.addEventListener("input", event => this.onInput(event));
    input.addEventListener("compositionend", () => this.onInput(null));
    input.addEventListener("focus", () => this.loadGate());
    input.addEventListener("keydown", event => this.onKeyDown(event));
    // (After `QueryQuoting`'s, which writes an open quote's end.)
    this.form?.addEventListener("submit", event => this.onSubmit(event));
    if (document.activeElement == input) {
      this.loadGate();
    }
    this.noteEntered();
  }

  static normalized(value) {
    return value.trim().replace(/\s+/g, " ");
  }

  async loadGate() {
    if (this.gate || this.gateLoading || !this.tree) {
      return;
    }
    this.gateLoading = true;
    try {
      const response = await fetch(`/${this.tree}/query-terms`);
      if (response.ok) {
        this.gate = new QueryGate(await response.json());
      }
    } catch (e) {
      console.error("query-frames: no query terms:", e);
    }
    this.gateLoading = false;
    // (What was typed meanwhile.)
    if (this.gate && this.input.value != this.pageValue) {
      this.onInput(null);
    }
  }

  onInput(event) {
    if (event?.isComposing) {
      return;
    }
    clearTimeout(this.timer);
    this.timer = null;
    if (!QueryResultsFrames.normalized(this.input.value)) {
      // (Clearing the box, as Escape does, but leaving it empty.)
      this.endSession({ restoreValue: false });
      return;
    }
    if (!this.gate) {
      this.loadGate();
      return;
    }
    const check = this.gate.check(this.input.value);
    if (this.stopped || !check.searchable) {
      return;
    }
    this.timer = setTimeout(() => {
      this.timer = null;
      this.request();
    }, QueryGate.delay(check.weight));
  }

  /** Load the box's query, unless it's loaded or loading already. */
  request() {
    if (this.loading) {
      this.more = true;
      return;
    }
    this.more = false;
    const query = QueryResultsFrames.normalized(this.input.value);
    if (!this.gate?.check(this.input.value).searchable ||
        query == (this.shown?.query ?? QueryResultsFrames.normalized(this.pageValue))) {
      return;
    }
    const url = this.formURL();
    if (!url) {
      return;
    }
    const frame = document.createElement("iframe");
    frame.className = "query-frame loading";
    frame.title = "Search results";
    const loading = { frame, query, loads: 0, ready: false };
    frame.addEventListener("queryframeready", () => this.onReady(loading));
    frame.addEventListener("load", () => this.onLoad(loading));
    // (Before it's in the page, so that its first load is the URL's, not
    // about:blank's.)
    frame.src = url;
    this.loading = loading;
    this.container().append(frame);
    this.searchBox?.classList.add("in-progress");
  }

  /** The URL the box's form submits to, as Enter would. */
  formURL() {
    if (!this.form) {
      return null;
    }
    const url = new URL(this.form.getAttribute("action"), document.location.href);
    url.search = new URLSearchParams(new FormData(this.form)).toString();
    return url.href;
  }

  container() {
    let container = document.getElementById("query-frames");
    if (!container) {
      container = document.createElement("div");
      container.id = "query-frames";
      document.body.append(container);
    }
    return container;
  }

  onLoad(loading) {
    loading.loads++;
    if (loading.loads == 1) {
      if (!loading.ready) {
        // (Not one of our pages, ex: a server's error's plain text.)
        this.stopped = true;
        this.say("Searching as you type stopped: press Enter to search.");
        this.onReady(loading);
      }
      return;
    }
    // (It went somewhere else: a link's or a script's navigation, which is
    // the whole window's.)
    if (this.shown === loading) {
      document.location.href = loading.frame.contentWindow.location.href;
    }
  }

  /** Say `text` in the box's bubble (see search.js's `Dxr.showBubble`), or nothing. */
  say(text) {
    const bubble = document.getElementById("query-bubble");
    if (!bubble) {
      return;
    }
    bubble.classList.remove("error", "warning");
    bubble.classList.add("info");
    bubble.textContent = text ?? "";
    bubble.style.display = text ? "block" : "none";
  }

  onReady(loading) {
    if (loading.ready) {
      return;
    }
    loading.ready = true;
    if (this.loading === loading) {
      this.loading = null;
    }
    if (!loading.frame.isConnected) {
      return;
    }
    this.searchBox?.classList.remove("in-progress");
    const previous = this.shown;
    this.shown = loading;
    loading.frame.classList.remove("loading");
    previous?.frame.remove();
    if (!this.session) {
      this.session = {
        id: `${Date.now()}-${Math.random()}`,
        pushed: false,
        baseUrl: withoutFragment(document.location.href),
        baseTitle: document.title,
        scroll: window.scrollY,
      };
    }
    document.documentElement.classList.add("query-frames-shown");
    const page = loading.frame.contentWindow;
    if (page?.document.title) {
      document.title = page.document.title;
    }
    // (Its URL, after `/search`'s redirect.)
    const url = page?.location.href;
    if (url && url != "about:blank") {
      const state = { queryFrames: this.session.id };
      if (this.session.pushed) {
        history.replaceState(state, "", url);
      } else {
        history.pushState(state, "", url);
        this.session.pushed = true;
      }
    }
    if (this.more) {
      this.request();
    }
  }

  /**
   * Remove the frames, and show the page again, as it was before typing:
   * its title, its scroll, and (`restoreValue`) the box's value, and its URL
   * (going back from the session's entry, unless `popped` already).
   */
  endSession({ restoreValue = true, popped = false } = {}) {
    clearTimeout(this.timer);
    this.timer = null;
    this.more = false;
    this.stopped = false;
    this.say(null);
    // (A loading frame's request is aborted, but its response cached
    // anyway; see nginx-setup.py's `/query` location.)
    document.getElementById("query-frames")?.remove();
    this.loading = this.shown = null;
    this.searchBox?.classList.remove("in-progress");
    if (restoreValue) {
      this.input.value = this.pageValue;
    }
    const session = this.session;
    this.session = null;
    if (!session) {
      return;
    }
    document.documentElement.classList.remove("query-frames-shown");
    document.title = session.baseTitle;
    window.scrollTo(window.scrollX, session.scroll);
    if (session.pushed && !popped) {
      this.popping = session.id;
      history.back();
    }
  }

  /**
   * Back (or forward) to another entry (see search.js's popstate handler,
   * which this goes before): from the session's entry to the page it's over,
   * the session ends, without reloading the page (and `endSession`'s Back);
   * else whether the page reloads is search.js's to say.
   */
  onPopState(event) {
    const url = withoutFragment(document.location.href);
    if (this.popping) {
      this.popping = null;
      contentUrl = url;
      return true;
    }
    if (this.session?.pushed && event.state?.queryFrames != this.session.id &&
        url == this.session.baseUrl) {
      this.endSession({ popped: true });
      contentUrl = url;
      return true;
    }
    return false;
  }

  /**
   * Enter: the session's results for what's typed, if they're shown, become
   * the page (their frame's URL, with any facets chosen in it), else the
   * form submits as it would (to the frame's page's URL otherwise); in the
   * session's entry, if it has one.
   */
  onSubmit(event) {
    clearTimeout(this.timer);
    this.timer = null;
    const query = QueryResultsFrames.normalized(this.input.value);
    let url = null;
    if (this.shown?.query == query) {
      url = this.shown.frame.contentWindow?.location.href;
    }
    sessionStorage.setItem(QueryResultsFrames.ENTERED_KEY, String(Date.now()));
    if (!url && !this.session?.pushed) {
      return;
    }
    event.preventDefault();
    url ??= this.formURL();
    if (this.session?.pushed) {
      document.location.replace(url);
    } else {
      document.location.assign(url);
    }
  }

  /**
   * A page that a query box's Enter made (within the last minute; see
   * `onSubmit`) is marked in its history entry, so that Escape in its box
   * goes back from it (see `onKeyDown`).
   */
  noteEntered() {
    const entered = Number(sessionStorage.getItem(QueryResultsFrames.ENTERED_KEY));
    if (!entered) {
      return;
    }
    sessionStorage.removeItem(QueryResultsFrames.ENTERED_KEY);
    if (Date.now() - entered < 60 * 1000) {
      history.replaceState({ ...history.state, queryEntered: true }, "");
    }
  }

  onKeyDown(event) {
    if (event.key != "Escape" || event.isComposing) {
      return;
    }
    if (this.session || this.loading || this.timer) {
      event.preventDefault();
      this.endSession();
    } else if (history.state?.queryEntered) {
      event.preventDefault();
      history.back();
    }
  }
}

// In a query box's frame of results (see settings.js, which marks it early):
// links go to the parent (the whole window), but for those to the page itself (ex: to a
// line of a result), and the frame says when its page is parsed and its
// scripts have run (by DOMContentLoaded, after deferred scripts, ex: the
// facets').  Elsewhere, a query box (in query mode: see search.js's `Dxr`)
// has results as one types.
if (document.documentElement.classList.contains("in-query-frame")) {
  document.addEventListener("click", event => {
    const link = event.target.closest?.("a[href]");
    if (!link || link.target) {
      return;
    }
    // (A link to the page itself is to a fragment of it, or a menu's.)
    if (withoutFragment(link.href) == withoutFragment(document.location.href)) {
      return;
    }
    // (Its parent being the page with the box, the whole window but in tests'
    // harness, whose frame the page is in.)
    link.target = "_parent";
  }, true);
  const ready = () => window.frameElement?.dispatchEvent(new CustomEvent("queryframeready"));
  if (document.readyState == "loading") {
    document.addEventListener("DOMContentLoaded", ready);
  } else {
    ready();
  }
} else {
  const input = document.getElementById("query");
  if (input?.form && !document.getElementById("path")) {
    window.QueryFrames = new QueryResultsFrames(input);
  }
}
