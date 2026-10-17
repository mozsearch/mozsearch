"use strict";

// The data API (see tools/src/data_api.rs): the trees' index data as JSON, by
// the pipeline-server, through nginx, of the tree as indexed or as a try push
// changes it (the trydelta tree's push 0123456789ab; see test_TryDelta.js),
// with strictly checked requests.

const API = "/trydelta/api/v1";
const PUSH = "0123456789ab";
const OLD_FROB = "_ZN8trydelta6Widget4FrobEi";
const NEW_FROB = "_ZN8trydelta6Widget4FrobEib";
// "Let widgets shrink".
const LANDED = "3b6f1c8f943932e006c9b03d8978879e05dfc4fc";

function doc() {
  return frame.contentDocument;
}

async function get(url, headers = {}) {
  const response = await fetch(url, { headers, cache: "no-store" });
  return { status: response.status, headers: response.headers, text: await response.text() };
}

async function getJson(url) {
  const { status, text } = await get(url);
  is(status, 200, `${url} is there`);
  return JSON.parse(text);
}

// (As test_TryDelta.js's.)
async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/${PUSH}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  return doc().querySelector(".explore-semantic-head a")?.getAttribute("href").split("/")[3];
}

add_task(async function test_DataApiRoutes() {
  const info = await getJson(`${API}/tree-info`);
  is(info.name, "trydelta", "The tree's info has its name");
  is(info.commit?.header?.includes("Let widgets shrink"), true, "and its indexed commit");
  is(info.commit?.summary, "Let widgets shrink", "and its summary");

  const config = await getJson(`${API}/config`);
  is(config.search_endpoint, "query", "The tree's configuration has its settings");
  is(config.linked_trees?.SPEC_, "tests", "all of them");
  ok(!("private" in config), "but not its private ones");

  const response = await get(`${API}/crossref?sym=${OLD_FROB}`);
  is(response.headers.get("content-type"), "application/json", "Crossref entries are JSON");
  is(response.headers.get("access-control-allow-origin"), "*", "for any origin");
  const entry = JSON.parse(response.text);
  ok(entry.defs?.some(result => result.path == "widget.cpp"), "A symbol's crossref entry has its definitions");
  ok(entry.meta, "and its structured information");
  const meta = await getJson(`${API}/crossref?sym=${OLD_FROB}&fields=meta`);
  is(Object.keys(meta).join(","), "meta", "Fields can be selected");
  is(await getJson(`${API}/crossref?sym=NotASymbol`), null, "Unknown symbols have none");

  const jumpref = await getJson(`${API}/jumpref?sym=${OLD_FROB}`);
  ok(jumpref?.jumps?.def?.startsWith("widget.cpp"), "A symbol's jumpref has its jumps");
  const ids = await getJson(`${API}/identifiers?q=trydelta::Widget::Frob&exact=true`);
  ok(ids.some(([sym]) => sym == OLD_FROB), "Identifiers have their symbols");
  const files = await getJson(`${API}/files?re=${encodeURIComponent("^widget\\.")}`);
  is(files.file_matches?.map(match => match.path).sort().join(","), "widget.cpp,widget.h",
     "Files can be searched");

  const source = await get(`${API}/source/widget.cpp`);
  ok(source.text.includes("void Widget::Frob(int aAmount)"), "Files' sources are there");
  is(source.headers.get("content-type"), "text/plain; charset=utf-8", "as text");
  const analysis = await get(`${API}/analysis/widget.cpp`);
  const records = analysis.text.trim().split("\n").map(line => JSON.parse(line));
  ok(records.some(record => record.sym?.split(",").includes(OLD_FROB)),
     "and their analysis records, one a line");

  const lines = await getJson(`${API}/changed-lines?spec=${LANDED}`);
  ok(lines.files?.["widget.cpp"]?.added?.length, "The lines a commit changes are there");

  const formatted = await getJson(`${API}/formatted-lines/widget.cpp`);
  ok(formatted.lines?.some(line => line.includes(OLD_FROB)),
     "Files' source listings' lines are there");
  const fileInfo = await getJson(`${API}/file-info/widget.cpp`);
  ok(fileInfo?.path_kind, "Files' path kinds are there");
  const recency = await getJson(`${API}/file-recency/widget.cpp`);
  is(typeof recency, "object", "Files' history digests are there");
  const journal = await getJson(`${API}/journal?kind=future&target=widget.cpp`);
  ok(Array.isArray(journal), "History journals are there");
  const linked = await getJson(`${API}/linked-crossref?tree=tests&sym=SPEC_example_unpoke`);
  ok(linked?.defs?.length, "Linked trees' crossref entries are there");
  const text = await getJson(`${API}/text?re=Frob&limit=10`);
  ok(text.by_file?.some(file => file.file == "widget.cpp"), "Text searches are there");
});

add_task(async function test_DataApiTryContext() {
  const head = await importPush();
  const info = await getJson(`${API}/tree-info?rev=${head}`);
  is(info.try_meta?.push, PUSH, "With a try push's head, the tree's info has the push's");

  is(await getJson(`${API}/crossref?sym=${NEW_FROB}`), null,
     "A symbol the push adds isn't the indexed tree's");
  const entry = await getJson(`${API}/crossref?sym=${NEW_FROB}&rev=${head}`);
  ok(entry?.defs?.some(result => result.path == "widget.cpp"), "but is in the push's context");
  is((await get(`${API}/source/wobbler.cpp`)).status, 404, "as a file it adds is");
  const source = await get(`${API}/source/wobbler.cpp?rev=${head}`);
  ok(source.status == 200 && source.text.includes("Wobble"), "with its source");
  is((await get(`${API}/tree-info?rev=${"0".repeat(40)}`)).status, 404,
     "A push without an analysis isn't there");

  // Try pushes' imports (the web-server's routes) and their pages' data.
  const status = await getJson(`${API}/try-status?push=${PUSH}`);
  is(status.state, "imported", "A try push's import's status is there");
  const push = await getJson(`${API}/try-push?push=${PUSH}`);
  is(push.semantic_head, head, "and the imported push's semantic head");
  ok(push.stack?.length, "and stack");
  const revs = push.stack.map(commit => commit.rev).join(",");
  const explore = await getJson(`${API}/explore?revs=${revs}&stack_url=try/${PUSH}`);
  is(explore.commits?.length, push.stack.length, "as an explore page's data");
  ok(explore.file_groups?.some(group => group.files.some(file => file.path == "wobbler.cpp")),
     "with the files it changes");
  const asked = await fetch(`${API}/try-import?push=${PUSH}`, { method: "POST", cache: "no-store" });
  is(asked.status, 200, "Asking for an imported push's import is fine");
  is((await asked.json()).state, "imported", "and says it's imported");
  is((await get(`${API}/try-import?push=${PUSH}`)).status, 405, "but only with a POST");
});

add_task(async function test_DataApiBadRequests() {
  // Paths escaping the tree (as encoded, since browsers resolve plain ".."
  // segments), absolute, with backslashes, or naming files that aren't the
  // tree's (its git checkout's), are turned away.
  for (const [path, expected] of [
    ["source/..%2Fwidget.cpp", 422],
    ["source/%2Fetc%2Fpasswd", 422],
    ["source/..%5C..%5Cetc%5Cpasswd", 422],
    ["source/%C0%AE%C0%AE%2Fwidget.cpp", 422],
    ["analysis/..%2Fwidget.cpp", 422],
    ["source/.git/config", 404],
    // (nginx resolves these, decoded, outside of the API's routes.)
    ["source/..%2F..%2F..%2Ftrydelta%2Fgit%2Fwidget.cpp", null],
    ["source/..%2F..%2F..%2F..%2F..%2F..%2F..%2Fetc%2Fpasswd", null],
    ["source/widget.cpp%00.txt", null],
  ]) {
    const { status, text } = await get(`${API}/${path}`);
    if (expected) {
      is(status, expected, `${path} is turned away`);
    } else {
      ok(status >= 400, `${path} is turned away (${status})`);
    }
    ok(!text.includes("root:") && !text.includes("[core]"), `without a file's contents`);
  }

  // Unknown, repeated, or malformed parameters.
  for (const query of [
    `crossref?sym=${OLD_FROB}&utm_source=x`,
    `crossref?sym=${OLD_FROB}&sym=${NEW_FROB}`,
    `crossref?sym=${OLD_FROB}&`,
    `crossref?sym=%E0%A4%A`,
    `identifiers?q=Frob&limit=lots`,
    `crossref?sym=${OLD_FROB}&rev=main`,
    `changed-lines?spec=HEAD`,
  ]) {
    const { status, text } = await get(`${API}/${query}`);
    is(status, 422, `${query} is a bad request`);
    ok(JSON.parse(text).error, "which says why");
  }
  is((await get(`${API}/nothing`)).status, 404, "Unknown routes aren't there");

  // Responses over the size cap.
  const big = await get(`${API}/source/widget.cpp?max_bytes=10`);
  is(big.status, 406, "A response over the caller's cap isn't acceptable");
  const body = JSON.parse(big.text);
  ok(body.bytes > 10 && body.max_bytes == 10, "which says how big it is");
  is((await get(`${API}/source/widget.cpp?max_bytes=100000`)).status, 200, "and the cap can be raised");

  is((await get("/searchfox-line-blame/api/v1/tree-info")).status, 404,
     "A tree whose data API is off has none");
});

// The API's requests have their own token buckets (see
// tools/src/admission.rs), and take less up front: a bot out of its pages'
// tokens (10, refilling one per 5 s) can still make 30 API requests at once
// (of a tenth of a token each, and their processing time).  (Bad ones, which
// nginx doesn't cache.)
add_task(async function test_DataApiAdmission() {
  const unique = Math.floor(Math.random() * 0xffff).toString(16);
  const headers = { "Fastly-Client-IP": `2001:db8:${unique}::1`, "User-Agent": "curl/8.5.0" };
  const page = i => `/trydelta/rev/0000000000000000000000000000000000000000/api-${unique}-${i}`;
  const statuses = (await Promise.all([...Array(13).keys()].map(i => get(page(i), headers))))
    .map(response => response.status);
  ok(statuses.includes(429), `The client's pages are turned away (${statuses})`);
  const apiStatuses = (await Promise.all([...Array(30).keys()].map(
    i => get(`${API}/jumpref?sym=${OLD_FROB}&unique=${unique}-${i}`, headers))))
    .map(response => response.status);
  is(apiStatuses.filter(status => status == 422).length, 30,
     `but its API requests all reach the API (${apiStatuses})`);
});
