"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 240 (it imports pushes)

// The carry-over of a tree's imported try pushes to the next index's web
// server (see the tools' try_carry_over.rs): the trydelta tree's web server's
// inventory of its imports, only for requests from inside (the indexer), and,
// as if it were the next index's web server, importing again the live set of
// an inventory posted to it (as cutover-gate.py does): c0ffee0c0ffe, asked for
// an hour ago, but not 0ddba11c0de0 (which no fixture has), a month ago, which
// the inventory keeps, for its page to say what it was.

const IMPORTED = "abcdef012345";
const RESTORED = "c0ffee0c0ffe";
const OLD = "0ddba11c0de0";

function doc() {
  return frame.contentDocument;
}

async function json(path, options = {}) {
  const response = await fetch(path, { cache: "no-store", ...options });
  return { status: response.status, body: response.ok ? await response.json() : null };
}

add_task(async function test_TryCarryOverInventory() {
  await TestUtils.loadPath(`/trydelta/try/hg/${IMPORTED}`);
  if (!doc().querySelector("#try-review")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector("#try-review"),
                           "The push is imported", 500, 240);
  }
  const { status, body } = await json("/trydelta/try-imports/inventory");
  is(status, 200, "The web server has an inventory of its try imports");
  const push = body.pushes.find(each => each.push == IMPORTED);
  ok(push, "which has an imported push");
  ok(push?.commits.some(commit => commit.phab == 8901 && commit.bugs.includes(4567)),
     `with its commits' Phabricator revisions and bugs (${JSON.stringify(push?.commits)})`);
  ok(push?.imported > 0 && push?.head.length == 40, "when it was imported, and its head");
  ok(push?.requested > 0 || push?.viewed > 0, "when it was last asked for or looked at");
  is(push?.semantic, "analyzed", "and its semantic analysis");

  const forwarded = await json("/trydelta/try-imports/inventory",
                               { headers: { "X-Forwarded-For": "192.0.2.1" } });
  is(forwarded.status, 403, "It's only for requests from inside, not through the load balancer");
});

add_task(async function test_TryCarryOverRestore() {
  const now = Math.floor(Date.now() / 1000);
  const inventory = {
    pushes: [
      { push: RESTORED, head: "", landed: false, commits: [], semantic: "none",
        requested: now - 3600 },
      { push: OLD, head: "", landed: false, semantic: "none", requested: now - 30 * 86400,
        commits: [{ rev: "0".repeat(40), summary: "Bug 1 - Do an old thing", bugs: [1] }] },
    ],
  };
  const posted = await json("/trydelta/try-imports/restore",
                            { method: "POST", body: JSON.stringify(inventory) });
  is(posted.status, 200, "An inventory posted to the web server");
  const states = status => Object.fromEntries(status.pushes.map(each => [each.push, each.state]));
  ok(RESTORED in states(posted.body) && !(OLD in states(posted.body)),
     `has its live set, the pushes asked for lately, imported again (${JSON.stringify(posted.body)})`);

  let status = posted.body;
  for (let i = 0; i < 240 && states(status)[RESTORED] != "imported"; i++) {
    await TestUtils.sleep(500);
    status = (await json("/trydelta/try-imports/restore-status")).body;
  }
  is(states(status)[RESTORED], "imported", "whose progress says when they're imported");
  ok(status.live >= 1 && status.queued == 0 && status.running == 0, `and how many (${JSON.stringify(status)})`);

  await TestUtils.loadPath(`/trydelta/try/hg/${RESTORED}`);
  ok(doc().querySelector("#try-review"), "The restored push's page is its review");

  const { body } = await json("/trydelta/try-imports/inventory");
  const old = body.pushes.find(each => each.push == OLD);
  is(old?.commits[0]?.summary, "Bug 1 - Do an old thing",
     "The inventory keeps a push carried over but not imported again, with what it was");
  ok(body.pushes.find(each => each.push == RESTORED)?.requested >= now - 3600,
     "and a restored push's activity, carried over");
});
