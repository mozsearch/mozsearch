"use strict";

add_task(async function test_CommitInfoRevision() {
  const response = await fetch("/searchfox/commit-info/HEAD");
  ok(response.ok, "HEAD commit info is available");
  const [head] = await response.json();
  ok(/^[0-9a-f]{40}$/.test(head.rev), "HEAD resolves to a full commit hash");

  const byHash = await fetch(`/searchfox/commit-info/${head.rev},${head.parent}`);
  ok(byHash.ok, "Commit info is available by hash");
  const [sameCommit, parent] = await byHash.json();
  is(sameCommit.rev, head.rev, "The resolved hash identifies HEAD");
  is(parent.rev, head.parent, "The parent has its own revision hash");
});
