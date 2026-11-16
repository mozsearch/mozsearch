"use strict";

// What a reviewer has marked of something under review (ex: a try push; see
// the `try-review:` view), kept in IndexedDB, and shared live with the other
// tabs showing it (see asuth-gecko-notes' try-review-ui-plan.md).
//
// The page registers the parts under review, a hierarchy (ex: patches, their
// files, the files' classes and functions, and their hunks), and the
// reviewer marks any of them: good, bad, looking at it, not theirs, or not
// looked at yet.  A part's mark is its own, if it has one, or else its
// nearest marked ancestor's (inherited: marking a class good marks what's in
// it good, unless the reviewer marked some of it otherwise), or else "not
// looked at yet".  Only what the reviewer clicked is recorded, with each
// change in a log, which undo walks back, and notes can be left on any part.
//
// Storage: the database "searchfox-review", whose store "reviews" holds one
// record per review, `{key, data, indices}`: `data` is the review's state
// (`marks` and `notes` by part, the change `log`, and `seq`, its count of
// changes), and `indices` are what it's of (ex: `["rev", HASH]`, `["bug",
// N]`), indexed (multiEntry) so that a bug's reviews can be found later.
// Each change is a readwrite transaction of its own (read, apply, write), so
// tabs can't lose each other's changes; the tab then posts it on the
// review's BroadcastChannel, and the others apply it without writing (or, if
// they missed one, by their `seq`, read the record again).

var ReviewState = (function () {
  const DB_NAME = "searchfox-review";
  const STORE = "reviews";
  // (The log's last changes kept, for undo.)
  const MAX_LOG = 500;

  // The marks, in the order of the icon strips.
  const MARKS = [
    { state: "good", icon: "✅", label: "Reviewed: good" },
    { state: "bad", icon: "❌", label: "Reviewed: needs changes" },
    { state: "looking", icon: "\u{1f441}️", label: "Looking at it" },
    { state: "notmine", icon: "\u{1f937}", label: "Not mine to review" },
    { state: "unseen", icon: "❓", label: "Not looked at yet" },
  ];
  const STATES = MARKS.map(mark => mark.state);

  function openDb() {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(DB_NAME, 1);
      request.onupgradeneeded = () => {
        const store = request.result.createObjectStore(STORE, { keyPath: "key" });
        store.createIndex("indices", "indices", { multiEntry: true });
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }

  function emptyData() {
    return { marks: {}, notes: {}, log: [], seq: 0 };
  }

  // Sets a part's field ("marks" or "notes") to `value`, or removes it if
  // it's null.
  function applyTo(data, change) {
    const fields = data[change.field];
    if (change.after === null || change.after === undefined) {
      delete fields[change.part];
    } else {
      fields[change.part] = change.after;
    }
  }

  class Review {
    /**
     * @param {String} key  the review's key (ex: "firefox-try/try/<rev>")
     * @param {Array} indices  what it's of, ex: [["rev", "abc"], ["bug", 1]]
     */
    constructor(key, indices = []) {
      this.key = key;
      this.indices = indices;
      this.data = emptyData();
      // Part id -> {parent, level, label}.
      this.parts = new Map();
      this.children = new Map();
      this.listeners = [];
      // Whether changes are kept (IndexedDB opened), and the last error.
      this.persistent = false;
      this.error = null;
      this.tab = Math.random().toString(36).slice(2);
      this.channel = null;
      // (The changes not written yet, in order: one transaction at a time.)
      this.writing = Promise.resolve();
    }

    /** Adds a part: its id, its parent's id (or null), level and label. */
    addPart(id, parent, level, label) {
      this.parts.set(id, { parent, level, label });
      if (parent) {
        if (!this.children.has(parent)) {
          this.children.set(parent, []);
        }
        this.children.get(parent).push(id);
      }
    }

    /** Loads the review's record, and listens to the other tabs'. */
    async open() {
      try {
        this.db = await openDb();
        const record = await this.read();
        if (record) {
          this.data = record.data;
        }
        this.persistent = true;
      } catch (e) {
        // (Ex: a private window without IndexedDB: the marks work, but
        // aren't kept.)
        this.error = e;
        console.warn("The review's marks can't be kept:", e);
      }
      if (typeof BroadcastChannel == "function") {
        this.channel = new BroadcastChannel(`searchfox-review:${this.key}`);
        this.channel.onmessage = event => this.received(event.data);
      }
      this.notify(null);
      return this;
    }

    /** Stops listening to the other tabs (after this tab's writes). */
    async close() {
      await this.writing;
      this.channel?.close();
      this.channel = null;
      this.db?.close();
      this.db = null;
    }

    read() {
      return new Promise((resolve, reject) => {
        const request = this.db.transaction(STORE).objectStore(STORE).get(this.key);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
    }

    // Writes a change: in one transaction, reads the record, applies it, and
    // puts it back; resolves to the record's new `seq`.
    write(change) {
      return new Promise((resolve, reject) => {
        const transaction = this.db.transaction(STORE, "readwrite");
        const store = transaction.objectStore(STORE);
        let seq = null;
        const request = store.get(this.key);
        request.onsuccess = () => {
          const record = request.result || { key: this.key, data: emptyData(), indices: [] };
          applyTo(record.data, change);
          record.data.log.push(change);
          if (record.data.log.length > MAX_LOG) {
            record.data.log.splice(0, record.data.log.length - MAX_LOG);
          }
          record.data.seq = seq = (record.data.seq || 0) + 1;
          record.indices = this.indices;
          store.put(record);
        };
        transaction.oncomplete = () => resolve(seq);
        transaction.onerror = () => reject(transaction.error);
        transaction.onabort = () => reject(transaction.error);
      });
    }

    // Makes a change here (at once), then keeps it and tells the other tabs.
    change(part, field, after) {
      const before = this.data[field][part] ?? null;
      const change = {
        id: `${this.tab}-${this.data.seq}-${Math.random().toString(36).slice(2, 8)}`,
        part, field, before, after, at: new Date().toISOString(), tab: this.tab,
      };
      return this.commit(change);
    }

    commit(change) {
      applyTo(this.data, change);
      this.data.log.push(change);
      if (this.data.log.length > MAX_LOG) {
        this.data.log.splice(0, this.data.log.length - MAX_LOG);
      }
      this.data.seq++;
      this.notify(change.part);
      if (!this.persistent) {
        this.channel?.postMessage({ change, seq: null });
        return Promise.resolve();
      }
      this.writing = this.writing.then(async () => {
        try {
          const seq = await this.write(change);
          // (Not back, if another tab's change, read meanwhile, came after.)
          this.data.seq = Math.max(this.data.seq, seq);
          this.channel?.postMessage({ change, seq });
        } catch (e) {
          this.error = e;
          console.warn("Couldn't keep a review mark:", e);
          this.notify(change.part);
        }
      });
      return this.writing;
    }

    // Another tab's change: applied, if it's the next one; else (a missed
    // change, or one made at the same time), the record is read again.
    async received({ change, seq }) {
      if (seq !== null && this.persistent && seq != this.data.seq + 1) {
        try {
          // (After this tab's own changes are written, so that the record
          // has them.)
          await this.writing;
          const record = await this.read();
          if (record) {
            this.data = record.data;
          }
        } catch (e) {
          this.error = e;
        }
        this.notify(null);
        return;
      }
      applyTo(this.data, change);
      this.data.log.push(change);
      this.data.seq = seq ?? this.data.seq + 1;
      this.notify(change.part);
    }

    /** Marks a part (one of `STATES`), or clears its mark (null). */
    mark(part, state) {
      if (state !== null && !STATES.includes(state)) {
        throw new Error(`Not a review mark: ${state}`);
      }
      const own = this.data.marks[part]?.state ?? null;
      if (own === state) {
        return Promise.resolve();
      }
      return this.change(part, "marks", state === null ? null : { state });
    }

    /** Leaves a note on a part (or removes it, if it's empty). */
    note(part, text) {
      text = (text || "").trim();
      const own = this.data.notes[part]?.text ?? "";
      if (own === text) {
        return Promise.resolve();
      }
      return this.change(part, "notes", text ? { text } : null);
    }

    /** The latest change not undone (the log's own undos excepted). */
    lastUndoable() {
      const undone = new Set();
      for (let i = this.data.log.length - 1; i >= 0; i--) {
        const change = this.data.log[i];
        if (change.undoes) {
          undone.add(change.undoes);
          continue;
        }
        if (!undone.has(change.id)) {
          return change;
        }
      }
      return null;
    }

    canUndo() {
      return this.lastUndoable() !== null;
    }

    /** Undoes the latest change (a change of its own, putting it back). */
    undo() {
      const last = this.lastUndoable();
      if (!last) {
        return Promise.resolve(null);
      }
      const change = {
        id: `${this.tab}-${this.data.seq}-undo`,
        part: last.part, field: last.field,
        before: this.data[last.field][last.part] ?? null, after: last.before,
        undoes: last.id, at: new Date().toISOString(), tab: this.tab,
      };
      return this.commit(change).then(() => last);
    }

    /** A part's own mark, or null. */
    own(part) {
      return this.data.marks[part]?.state ?? null;
    }

    /**
     * A part's mark: `{state, from}`, where `from` is the part whose mark it
     * is (itself, or the ancestor it inherits it from), or null for parts
     * with none ("unseen").
     */
    effective(part) {
      for (let id = part; id; id = this.parts.get(id)?.parent) {
        const own = this.own(id);
        if (own) {
          return { state: own, from: id };
        }
      }
      return { state: "unseen", from: null };
    }

    /** A part's note's text, or "". */
    noteOf(part) {
      return this.data.notes[part]?.text ?? "";
    }

    /** The parts under a part (not it), depth first. */
    descendants(part) {
      const out = [];
      const stack = [...(this.children.get(part) || [])].reverse();
      while (stack.length) {
        const id = stack.pop();
        out.push(id);
        stack.push(...[...(this.children.get(id) || [])].reverse());
      }
      return out;
    }

    /**
     * The explicit marks under a part, by state (ex: how many of a class's
     * methods are marked bad), and its notes.
     */
    counts(part) {
      const counts = Object.fromEntries(STATES.map(state => [state, 0]));
      let notes = 0;
      for (const id of this.descendants(part)) {
        const own = this.own(id);
        if (own) {
          counts[own]++;
        }
        if (this.noteOf(id)) {
          notes++;
        }
      }
      return { counts, notes };
    }

    /** The parts of a level whose marks (effective) are a state. */
    partsWith(level, state) {
      const out = [];
      for (const [id, info] of this.parts) {
        if (info.level == level && this.effective(id).state == state) {
          out.push(id);
        }
      }
      return out;
    }

    /** Calls `listener(part)` after each change (part null: anything). */
    subscribe(listener) {
      this.listeners.push(listener);
    }

    notify(part) {
      for (const listener of this.listeners) {
        try {
          listener(part);
        } catch (e) {
          console.error(e);
        }
      }
    }
  }

  // ## Icon strips

  /**
   * A part's strip of marks: a button per mark (pressed if it's the part's,
   * with an arrow if it's inherited), the counts of the marks under it, and
   * a note button.  Clicking the part's own mark again clears it.
   */
  function strip(review, part) {
    const box = document.createElement("span");
    box.className = "review-strip";
    box.dataset.part = part;
    box.setAttribute("role", "group");
    box.setAttribute("aria-label", "Review marks");
    for (const mark of MARKS) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = `review-mark review-mark-${mark.state}`;
      button.dataset.state = mark.state;
      const icon = document.createElement("span");
      icon.className = "review-mark-icon";
      icon.textContent = mark.icon;
      const count = document.createElement("span");
      count.className = "review-mark-count";
      button.append(icon, count);
      button.addEventListener("click", event => {
        event.stopPropagation();
        review.mark(part, review.own(part) == mark.state ? null : mark.state);
      });
      box.append(button);
    }
    const note = document.createElement("button");
    note.type = "button";
    note.className = "review-note-button";
    note.textContent = "✏️";
    note.addEventListener("click", event => {
      event.stopPropagation();
      editNote(review, part, note);
    });
    box.append(note);
    updateStrip(review, box);
    return box;
  }

  function updateStrip(review, box) {
    const part = box.dataset.part;
    const { state, from } = review.effective(part);
    const { counts, notes } = review.counts(part);
    const label = review.parts.get(part)?.label ?? part;
    for (const button of box.querySelectorAll(".review-mark")) {
      const mark = MARKS.find(m => m.state == button.dataset.state);
      const pressed = mark.state == state;
      const inherited = pressed && from !== null && from != part;
      button.setAttribute("aria-pressed", pressed ? "true" : "false");
      button.classList.toggle("review-mark-inherited", inherited);
      const below = counts[mark.state];
      button.querySelector(".review-mark-count").textContent = below ? String(below) : "";
      let title = `${mark.label}: ${label}`;
      if (inherited) {
        title += ` (from ${review.parts.get(from)?.label ?? from})`;
      } else if (pressed && from == part) {
        title += " (click again to clear)";
      }
      if (below) {
        title += `; ${below} marked so in it`;
      }
      button.title = title;
    }
    const note = box.querySelector(".review-note-button");
    const text = review.noteOf(part);
    note.classList.toggle("review-note-present", !!text);
    note.title = text ? `Your note: ${text}` : `Leave a note on ${label}`;
    if (notes) {
      note.title += ` (${notes} more in it)`;
    }
  }

  // A note's editor, in a popup by its button: Enter (or leaving it) keeps
  // the note, Escape cancels.
  function editNote(review, part, button) {
    document.querySelector(".review-note-editor")?.remove();
    const editor = document.createElement("div");
    editor.className = "review-note-editor";
    const area = document.createElement("textarea");
    area.value = review.noteOf(part);
    area.placeholder = "A note to yourself (kept in this browser)";
    area.setAttribute("aria-label", `Note on ${review.parts.get(part)?.label ?? part}`);
    editor.append(area);
    const rect = button.getBoundingClientRect();
    editor.style.left = `${Math.max(4, rect.left + window.scrollX - 200)}px`;
    editor.style.top = `${rect.bottom + window.scrollY + 4}px`;
    document.body.append(editor);
    let done = false;
    const close = keep => {
      if (done) {
        return;
      }
      done = true;
      if (keep) {
        review.note(part, area.value);
      }
      editor.remove();
      button.focus();
    };
    area.addEventListener("keydown", event => {
      if (event.key == "Escape") {
        event.preventDefault();
        close(false);
      } else if (event.key == "Enter" && !event.shiftKey) {
        event.preventDefault();
        close(true);
      }
    });
    area.addEventListener("blur", () => close(true));
    area.focus();
  }

  /**
   * Keeps a page's strips (`.review-strip[data-part]`) up to date, and makes
   * Ctrl+Z (or Cmd+Z), outside of text fields, undo the latest change.
   */
  function bind(review, root = document) {
    review.subscribe(() => {
      for (const box of root.querySelectorAll(".review-strip[data-part]")) {
        updateStrip(review, box);
      }
    });
    document.addEventListener("keydown", event => {
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey &&
          event.key.toLowerCase() == "z" &&
          !event.target.closest?.("input, textarea, [contenteditable]")) {
        event.preventDefault();
        review.undo();
      }
    });
  }

  return { Review, MARKS, STATES, strip, updateStrip, bind, DB_NAME, STORE };
})();
