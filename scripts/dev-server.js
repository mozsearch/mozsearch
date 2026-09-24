#!/usr/bin/env node
// Local development server for hacking on the searchfox front-end without
// running the indexer.
//
// - /<tree>/static/* is served from this checkout's static/ directory.
// - /<tree>/source/<file> is rendered locally with tools' output-file, using
//   the file contents from GitHub and the analysis data from searchfox.org,
//   both at the revision currently indexed on searchfox.org.
// - Everything else (directory listings, search, blame, ...) is proxied to
//   searchfox.org.
//
// The tools are rebuilt (if needed) before each render, so changes to the Rust
// code or to tools/templates show up on reload.
//
// Per-file info (test info, bugzilla components, ...) and jumpref are built by
// running crossref on the files downloaded so far, with the per-file info
// inputs downloaded from Taskcluster.  So "Go to definition" only knows about
// symbols defined in files that have been viewed.  Per-file info can be
// overridden in tools/target/dev-server/per-file-info.json, e.g.:
//   {"dom/base/test/test_bug5141.html": {"info": {"test": {"skip_if": "os == 'win'"}}}}
//
// Usage: node scripts/dev-server.js [port]

const { spawn } = require("child_process");
const fs = require("fs");
const http = require("http");
const path = require("path");
const zlib = require("zlib");

const PORT = parseInt(process.argv[2] || process.env.PORT || "8000", 10);
const TREE = process.env.TREE || "firefox-main";
const GITHUB_REPO = process.env.GITHUB_REPO || "mozilla-firefox/firefox";
const UPSTREAM = "https://searchfox.org";
// searchfox.org serves a JS challenge to some non-browser user agents.
const USER_AGENT = "Mozilla/5.0 (searchfox dev-server) Firefox/140.0";

const MOZSEARCH = path.resolve(__dirname, "..");
const TOOLS = path.join(MOZSEARCH, "tools");
const OUTPUT_FILE = path.join(TOOLS, "target/release/output-file");
const CROSSREF = path.join(TOOLS, "target/release/crossref");
const TASKCLUSTER_INDEX =
  "https://firefox-ci-tc.services.mozilla.com/api/index/v1/task/gecko.v2.mozilla-central.latest";
// Inputs for per-file info, see config_defaults/per-file-info.toml.
const PER_FILE_INFO_INPUTS = {
  "test-info-all-tests.json":
    "source.test-info-all/artifacts/public/test-info-all-tests.json",
  "bugzilla-components.json":
    "source.source-bugzilla-info/artifacts/public/components-normalized.json",
  "wpt-metadata-summary.json":
    "source.source-wpt-metadata-summary/artifacts/public/summary.json",
};
const DATA = path.join(TOOLS, "target/dev-server");
const OVERRIDES = path.join(DATA, "per-file-info.json");

const CONTENT_TYPES = {
  ".css": "text/css",
  ".js": "text/javascript",
  ".html": "text/html",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".eot": "application/vnd.ms-fontobject",
  ".txt": "text/plain",
};

let currentRev = null;
let currentRevTime = 0;
const REV_TTL_MS = 10 * 60 * 1000;

function upstreamFetch(urlPath, options = {}) {
  return fetch(UPSTREAM + urlPath, {
    ...options,
    headers: { ...options.headers, "User-Agent": USER_AGENT },
    redirect: "manual",
  });
}

// The revision currently indexed on searchfox.org, found in the permalink of a
// rendered page.  When it changes, previously downloaded files are discarded.
async function getRev() {
  if (currentRev && Date.now() - currentRevTime < REV_TTL_MS) {
    return currentRev;
  }
  const response = await upstreamFetch(`/${TREE}/source/moz.build`);
  const html = await response.text();
  const match = html.match(new RegExp(`/${TREE}/rev/([0-9a-f]{40})/`));
  if (!match) {
    throw new Error(`Couldn't find the indexed revision on ${UPSTREAM}`);
  }
  if (match[1] != currentRev) {
    console.log(`Indexed revision: ${match[1]}`);
    fs.rmSync(DATA + "/tree", { recursive: true, force: true });
    currentRev = match[1];
  }
  currentRevTime = Date.now();
  return currentRev;
}

function treeDir(...parts) {
  return path.join(DATA, "tree", ...parts);
}

function writeFile(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, data);
}

function writeConfig() {
  const config = {
    mozsearch_path: MOZSEARCH,
    config_repo: treeDir("config-repo"),
    default_tree: TREE,
    trees: {
      [TREE]: {
        priority: 1000,
        on_error: "continue",
        cache: "everything",
        index_path: treeDir("index"),
        files_path: treeDir("files"),
        objdir_path: treeDir("objdir"),
        git_branch: "main",
        github_repo: `https://github.com/${GITHUB_REPO}`,
        hg_root: "https://hg.mozilla.org/mozilla-central",
        wpt_root: "testing/web-platform",
        codesearch_path: "",
        codesearch_port: 0,
      },
    },
  };
  writeFile(treeDir("config.json"), JSON.stringify(config, null, 2));
  fs.mkdirSync(treeDir("config-repo"), { recursive: true });
}

async function fetchPerFileInfoInputs() {
  for (const [name, artifact] of Object.entries(PER_FILE_INFO_INPUTS)) {
    const file = treeDir("index", name);
    if (fs.existsSync(file)) {
      continue;
    }
    console.log(`Downloading ${name}`);
    const response = await fetch(`${TASKCLUSTER_INDEX}.${artifact}`);
    if (response.ok) {
      writeFile(file, Buffer.from(await response.arrayBuffer()));
    } else {
      console.error(`Failed to download ${name}: ${response.status}`);
    }
  }
}

function isDownloaded(filePath) {
  return !!fs.statSync(treeDir("files", filePath), {
    throwIfNoEntry: false,
  })?.isFile();
}

// Download the source and analysis of a file if we don't have them yet.
// Returns false if the path isn't a file at the indexed revision.
async function fetchFile(rev, filePath) {
  if (isDownloaded(filePath)) {
    return true;
  }
  const sourceFile = treeDir("files", filePath);
  const encoded = filePath.split("/").map(encodeURIComponent).join("/");
  const source = await fetch(
    `https://raw.githubusercontent.com/${GITHUB_REPO}/${rev}/${encoded}`
  );
  if (!source.ok) {
    return false;
  }
  const analysis = await upstreamFetch(`/${TREE}/raw-analysis/${encoded}`);
  writeFile(
    treeDir("index/analysis", filePath),
    analysis.ok ? Buffer.from(await analysis.arrayBuffer()) : ""
  );
  writeFile(sourceFile, Buffer.from(await source.arrayBuffer()));
  return true;
}

function readOverrides() {
  try {
    return JSON.parse(fs.readFileSync(OVERRIDES, "utf8"));
  } catch (e) {
    if (e.code != "ENOENT") {
      console.error(`Ignoring ${OVERRIDES}: ${e.message}`);
    }
    return {};
  }
}

function listFiles(dir, prefix = "") {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory()
      ? listFiles(path.join(dir, entry.name), prefix + entry.name + "/")
      : [prefix + entry.name]
  );
}

// Run crossref on all the downloaded files, to generate jumpref and per-file
// info.
async function crossref() {
  const files = listFiles(treeDir("files")).sort();
  const dirs = new Set();
  for (const file of files) {
    for (let dir = path.dirname(file); dir != "."; dir = path.dirname(dir)) {
      dirs.add(dir);
    }
  }
  const index = treeDir("index");
  // crossref skips files whose description it fails to write.
  for (const dir of dirs) {
    fs.mkdirSync(path.join(index, "description", dir), { recursive: true });
  }
  writeFile(path.join(index, "all-files"), files.join("\n") + "\n");
  writeFile(path.join(index, "all-dirs"), [...dirs].sort().join("\n") + "\n");
  await run(CROSSREF, [
    treeDir("config.json"),
    TREE,
    path.join(index, "all-files"),
    "2",
  ]);
  fs.renameSync(
    path.join(index, "concise-per-file-info.json"),
    path.join(index, "concise-per-file-info.crossref.json")
  );
}

function isPerFileInfoStale() {
  const output = fs.statSync(
    treeDir("index/concise-per-file-info.crossref.json"),
    { throwIfNoEntry: false }
  );
  const config = fs.statSync(
    path.join(MOZSEARCH, "config_defaults/per-file-info.toml")
  );
  return !output || output.mtimeMs < config.mtimeMs;
}

// Apply per-file-info.json on top of the crossref generated per-file info.
function writePerFileInfo() {
  const concise = JSON.parse(
    fs.readFileSync(treeDir("index/concise-per-file-info.crossref.json"))
  );
  for (const [file, override] of Object.entries(readOverrides())) {
    if (concise[file]) {
      concise[file] = {
        ...concise[file],
        ...override,
        info: { ...concise[file].info, ...override.info },
      };
    }
  }
  writeFile(
    treeDir("index/concise-per-file-info.json"),
    JSON.stringify(concise)
  );
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, options);
    let output = "";
    child.stdout.on("data", data => (output += data));
    child.stderr.on("data", data => (output += data));
    if (options.input !== undefined) {
      child.stdin.end(options.input);
    }
    child.on("error", reject);
    child.on("close", code => {
      if (code) {
        reject(new Error(`${command} ${args.join(" ")} failed:\n\n${output}`));
      } else {
        resolve(output);
      }
    });
  });
}

async function render(filePath) {
  await run(
    "cargo",
    ["build", "--release", "--bin", "output-file", "--bin", "crossref"],
    { cwd: TOOLS }
  );
  const rev = await getRev();
  writeConfig();
  await fetchPerFileInfoInputs();
  const isNew = !isDownloaded(filePath);
  if (!(await fetchFile(rev, filePath))) {
    return null;
  }
  if (isNew || isPerFileInfoStale()) {
    await crossref();
  }
  writePerFileInfo();
  const output = treeDir("index/file", filePath) + ".gz";
  fs.mkdirSync(path.dirname(output), { recursive: true });
  const log = await run(
    OUTPUT_FILE,
    [treeDir("config.json"), TREE, "none", "none", "-"],
    { input: filePath + "\n" }
  );
  if (!fs.existsSync(output) || !fs.statSync(output).size) {
    throw new Error(`output-file didn't render ${filePath}:\n\n${log}`);
  }
  return zlib.gunzipSync(fs.readFileSync(output));
}

// Renders share the per-file info files, so run them one at a time.
let renderQueue = Promise.resolve();
function queueRender(filePath) {
  const result = renderQueue.then(() => render(filePath));
  renderQueue = result.catch(() => {});
  return result;
}

function serveStatic(res, relPath) {
  const staticRoot = path.join(MOZSEARCH, "static");
  const file = path.join(staticRoot, relPath);
  if (!file.startsWith(staticRoot + path.sep) || !fs.existsSync(file)) {
    res.writeHead(404).end("Not found");
    return;
  }
  res.writeHead(200, {
    "Content-Type":
      CONTENT_TYPES[path.extname(file)] || "application/octet-stream",
    "Cache-Control": "no-store",
  });
  fs.createReadStream(file).pipe(res);
}

async function proxy(req, res) {
  const body = ["GET", "HEAD"].includes(req.method)
    ? undefined
    : await new Promise(resolve => {
        const chunks = [];
        req.on("data", chunk => chunks.push(chunk));
        req.on("end", () => resolve(Buffer.concat(chunks)));
      });
  const headers = {};
  for (const name of ["accept", "content-type"]) {
    if (req.headers[name]) {
      headers[name] = req.headers[name];
    }
  }
  const response = await upstreamFetch(req.url, {
    method: req.method,
    headers,
    body,
  });
  const responseHeaders = {};
  for (const [name, value] of response.headers) {
    // fetch() already decompressed the body.
    if (!["content-encoding", "content-length", "transfer-encoding",
          "connection"].includes(name)) {
      responseHeaders[name] = value;
    }
  }
  if (responseHeaders.location?.startsWith(UPSTREAM)) {
    responseHeaders.location = responseHeaders.location.slice(UPSTREAM.length);
  }
  res.writeHead(response.status, responseHeaders);
  res.end(Buffer.from(await response.arrayBuffer()));
}

async function handle(req, res) {
  const url = new URL(req.url, "http://localhost");
  const pathname = decodeURIComponent(url.pathname);

  if (pathname == "/") {
    res.writeHead(302, { Location: `/${TREE}/source/` }).end();
    return;
  }

  let match = pathname.match(/^(?:\/[^/]+)?\/static\/(.+)$/);
  if (match) {
    serveStatic(res, match[1]);
    return;
  }

  match = pathname.match(new RegExp(`^/${TREE}/source/(.+[^/])$`));
  if (match && req.method == "GET") {
    const html = await queueRender(match[1]);
    if (html) {
      console.log(`Rendered ${match[1]}`);
      res.writeHead(200, {
        "Content-Type": "text/html; charset=utf-8",
        "Cache-Control": "no-store",
      });
      res.end(html);
      return;
    }
  }

  await proxy(req, res);
}

http
  .createServer((req, res) => {
    handle(req, res).catch(e => {
      console.error(e);
      if (!res.headersSent) {
        res.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
      }
      res.end(e.stack || String(e));
    });
  })
  .listen(PORT, () => {
    console.log(`Listening on http://localhost:${PORT}/${TREE}/source/`);
  });
