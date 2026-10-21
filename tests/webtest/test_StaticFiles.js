"use strict";

// The static files (scripts, styles, icons, and vendored libraries like d3):
// served by searchfox itself, rather than CDNs, cached for 2 minutes and then
// revalidated (see scripts/nginx-setup.py's `static_cache`), so that a
// server's update is seen within 2 minutes.

add_task(async function test_StaticFilesCaching() {
  for (const path of ["/searchfox/static/js/d3.v7.min.js", "/static/css/mozsearch.css"]) {
    const response = await fetch(path, { cache: "no-store" });
    is(response.status, 200, `${path} is there`);
    const cacheControl = response.headers.get("Cache-Control") ?? "";
    ok(cacheControl.includes("max-age=120") && cacheControl.includes("must-revalidate"),
       `${path} is cached for 2 minutes, then revalidated (${cacheControl})`);
    ok(response.headers.get("ETag"), `${path} has an ETag to revalidate it by`);
  }
});

add_task(async function test_StaticFilesAreLocal() {
  // (A page with d3's chart.)
  await TestUtils.loadQuery("searchfox", "token-timeline:structopt");
  const doc = frame.contentDocument;
  const urls = [...doc.querySelectorAll("script[src]")].map(script => script.src)
    .concat([...doc.querySelectorAll("link[href]")].map(link => link.href));
  ok(urls.some(url => url.endsWith("/static/js/d3.v7.min.js")), "The page loads d3");
  for (const url of urls) {
    is(new URL(url).origin, frame.contentWindow.location.origin, `${url} is searchfox's`);
  }
});
