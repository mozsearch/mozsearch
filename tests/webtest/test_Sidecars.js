"use strict";

// A /rev/ page, and its token-centric blame data, are written by the
// web-server as sidecar files (see tools/src/sidecars.rs), which nginx serves
// from disk (with an ETag, which nginx doesn't add to the web-server's
// responses), falling back to the web-server, which writes what it generates
// too.
add_task(async function test_SidecarFiles() {
  // (A revision of a file whose page and sidecars other tests don't request:
  // the parent of its newest change.)
  await TestUtils.loadPath("/searchfox/source/scripts/nginx-setup.py");
  const timeline = frame.contentWindow.FileTimeline;
  const newest = timeline.commits[timeline.commits.length - 1];
  const headers = await (await frame.contentWindow.fetch(`${timeline.info.dataUrl}/commits.json`)).json();
  const page = `/searchfox/rev/${headers[newest.index].parent}/scripts/nginx-setup.py`;

  // (Not from the browser's cache, which keeps the web-server's responses for
  // a couple of minutes.)
  const firstPage = await frame.contentWindow.fetch(page, { cache: "no-store" });
  ok(firstPage.ok && !firstPage.headers.get("etag"), "The page comes from the web-server the first time");
  const firstPageText = await firstPage.text();
  const url = firstPageText.match(/"dataUrl":"([^"]+)"/)?.[1];
  ok(url?.includes("/rev-hyperblame/"), "The page's blame data is the web-server's");

  const chunk = await frame.contentWindow.fetch(`${url}/lines-0.json`);
  ok(chunk.ok, "Its first chunk is there");
  ok(chunk.headers.get("etag"), "served from the sidecar file generating the page wrote");

  // (Before the page's scripts request it.)
  const first = await frame.contentWindow.fetch(`${url}/commits.json`);
  ok(first.ok && !first.headers.get("etag"), "Its commits.json comes from the web-server the first time");
  const second = await frame.contentWindow.fetch(`${url}/commits.json`);
  ok(second.headers.get("etag"), "and then from the sidecar file it wrote");
  is(JSON.stringify(await second.json()), JSON.stringify(await first.json()), "which has the same data");

  await TestUtils.loadPath(page);
  const win = frame.contentWindow;
  is(win.BLAME_INFO.dataUrl, url, "The page has that blame data");
  const secondPage = await win.fetch(page, { cache: "no-store" });
  ok(secondPage.headers.get("etag"), "and comes from the sidecar file it wrote");
  is(secondPage.headers.get("content-type"), "text/html", "as HTML");
  is(await secondPage.text(), firstPageText, "which is the same page");
  const withQuery = await win.fetch(`${page}?exactly`, { cache: "no-store" });
  ok(withQuery.ok && !withQuery.headers.get("etag"), "but not for requests with queries");
});
