"use strict";

// The revisions of revision URLs are (maybe abbreviated) commit ids, or ref
// names, which redirect to their commits' ids, but none of git's other
// revision syntax.

add_task(async function test_RefNames() {
  for (const [url, expected] of [
    ["/tests/rev/main/README.md", /^\/tests\/rev\/[0-9a-f]{40}\/README\.md$/],
    ["/tests/diff/main/README.md", /^\/tests\/diff\/[0-9a-f]{40}\/README\.md$/],
    ["/tests/commit/main", /^\/tests\/commit\/[0-9a-f]{40}$/],
    // (A ref name with slashes.)
    ["/tests/rev/refs/heads/main/README.md", /^\/tests\/rev\/[0-9a-f]{40}\/README\.md$/],
  ]) {
    const response = await fetch(url, { cache: "no-store" });
    ok(response.ok, `${url} is a page`);
    const redirected = new URL(response.url).pathname;
    ok(response.redirected && expected.test(redirected),
       `${url} redirects to its commit's id: ${redirected}`);
  }

  const response = await fetch("/tests/rev/no-such-branch/README.md", { cache: "no-store" });
  ok(!response.ok, "A name which isn't a ref's isn't a revision");
});

add_task(async function test_RevisionSyntax() {
  for (const rev of ["main~1", "main^", "HEAD@{now}", "main^{/Add}"]) {
    const response = await fetch(`/tests/rev/${encodeURIComponent(rev)}/README.md`,
                                 { cache: "no-store" });
    ok(!response.ok, `${rev} isn't a revision`);
  }
});
