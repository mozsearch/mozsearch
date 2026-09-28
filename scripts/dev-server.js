#!/usr/bin/env node
// Local development server for hacking on the searchfox front-end without
// running the indexer.
//
// - /<tree>/static/* is served from this checkout's static/ directory.
// - /<tree>/source/<file> is rendered locally with tools' output-file, using
//   the file contents from GitHub and the analysis data from searchfox.org,
//   both at the revision currently indexed on searchfox.org.
// - /<tree>/source/<directory> is rendered locally with searchfox-tool, using
//   the list of files from searchfox.org.  Files that haven't been viewed yet
//   are represented by empty placeholders of the right size.
// - Everything else (search, blame, ...) is proxied to searchfox.org.
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
const SEARCHFOX_TOOL = path.join(TOOLS, "target/release/searchfox-tool");
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
  "xpcshell-issues.json":
    "source.test-info-xpcshell-timings/artifacts/public/xpcshell-issues.json",
  "mochitest-issues.json":
    "source.test-info-mochitest-timings/artifacts/public/mochitest-issues.json",
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
  const script = path.join(MOZSEARCH, "scripts/summarize-test-results.py");
  const issues = ["xpcshell", "mochitest"].map(harness =>
    treeDir(`index/${harness}-issues.json`)
  );
  if (isOlderThan(treeDir("index/test-results.json"), script, ...issues)) {
    await run("python3", [script, treeDir("index/test-results.json"), ...issues]);
  }
}

// The WPT manifests are in a tarball, extracted like the config repo's
// fetch-tc-artifacts.sh does.
async function fetchWptManifests() {
  if (fs.existsSync(treeDir("index/wpt-manifest.json"))) {
    return;
  }
  console.log("Downloading wpt-manifests.tar.gz");
  const response = await fetch(
    `${TASKCLUSTER_INDEX}.source.manifest-upload/artifacts/public/manifests.tar.gz`
  );
  if (!response.ok) {
    console.error(`Failed to download the WPT manifests: ${response.status}`);
    return;
  }
  const tarball = treeDir("index/wpt-manifests.tar.gz");
  const extracted = treeDir("index/wpt-manifests");
  writeFile(tarball, Buffer.from(await response.arrayBuffer()));
  fs.mkdirSync(extracted, { recursive: true });
  await run("tar", ["xzf", tarball, "-C", extracted]);
  fs.renameSync(
    path.join(extracted, "mozilla/meta/MANIFEST.json"),
    treeDir("index/wpt-mozilla-manifest.json")
  );
  fs.renameSync(
    path.join(extracted, "meta/MANIFEST.json"),
    treeDir("index/wpt-manifest.json")
  );
  // Make crossref ingest them.
  fs.rmSync(treeDir("index/concise-per-file-info.crossref.json"), {
    force: true,
  });
}

// Files listed in a directory but not viewed yet are empty placeholders, which
// don't have analysis data.
function isDownloaded(filePath) {
  return !!fs.statSync(treeDir("index/analysis", filePath), {
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

// Returns the files and directories under `dir`, recursively.
function listTree(dir, prefix = "", result = { files: [], dirs: [] }) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      result.dirs.push(prefix + entry.name);
      listTree(path.join(dir, entry.name), prefix + entry.name + "/", result);
    } else {
      result.files.push(prefix + entry.name);
    }
  }
  return result;
}

// Run crossref on all the downloaded and listed files, to generate jumpref and
// per-file info.
async function crossref() {
  const { files, dirs } = listTree(treeDir("files"));
  files.sort();
  dirs.sort();
  const index = treeDir("index");
  // crossref skips files whose description it fails to write.
  for (const dir of dirs) {
    fs.mkdirSync(path.join(index, "description", dir), { recursive: true });
  }
  const lines = paths => paths.map(p => p + "\n").join("");
  writeFile(path.join(index, "all-files"), lines(files));
  writeFile(path.join(index, "all-dirs"), lines(dirs));
  writeFile(path.join(index, "analysis-files"), lines(files.filter(isDownloaded)));
  await run(CROSSREF, [
    treeDir("config.json"),
    TREE,
    path.join(index, "analysis-files"),
    "2",
  ]);
  fs.renameSync(
    path.join(index, "concise-per-file-info.json"),
    path.join(index, "concise-per-file-info.crossref.json")
  );
}

// Whether `output` is missing or older than any of the existing `inputs`.
function isOlderThan(output, ...inputs) {
  const outputStat = fs.statSync(output, { throwIfNoEntry: false });
  return (
    !outputStat ||
    inputs.some(
      input =>
        fs.statSync(input, { throwIfNoEntry: false })?.mtimeMs >
        outputStat.mtimeMs
    )
  );
}

function isPerFileInfoStale() {
  return isOlderThan(
    treeDir("index/concise-per-file-info.crossref.json"),
    path.join(MOZSEARCH, "config_defaults/per-file-info.toml"),
    treeDir("index/test-results.json")
  );
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

// Create placeholders for the files and directories of a directory listing
// on searchfox.org.  Returns false if the directory doesn't exist, and whether
// new placeholders were created otherwise.
async function listDirectory(dirPath) {
  const encoded = dirPath.split("/").map(encodeURIComponent).join("/");
  const response = await upstreamFetch(`/${TREE}/source/${encoded}/`);
  if (!response.ok) {
    return null;
  }
  const html = await response.text();
  const rows = html.matchAll(
    new RegExp(
      `<td class="name"><a href="/${TREE}/source/([^"]+)" class="[^"]*mimetype-icon-([^" ]+)">` +
        `.*?<td class="size"><a [^>]*>(\\d*)</a>`,
      "gs"
    )
  );
  let changed = false;
  for (const [, href, icon, size] of rows) {
    const childPath = href.split("/").map(decodeURIComponent).join("/");
    const file = treeDir("files", childPath);
    if (fs.existsSync(file)) {
      continue;
    }
    changed = true;
    if (icon == "folder") {
      fs.mkdirSync(file, { recursive: true });
    } else {
      writeFile(file, "");
      fs.truncateSync(file, parseInt(size || "0", 10));
    }
  }
  return changed;
}

function escapeRegExp(string) {
  return string.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function renderDirectory(dirPath) {
  const changed = await listDirectory(dirPath);
  if (changed === null) {
    return null;
  }
  if (changed || isPerFileInfoStale()) {
    await crossref();
  }
  writePerFileInfo();
  const pathre = dirPath
    ? `^${escapeRegExp(dirPath)}(/[^/]+)?$`
    : "^[^/]+$";
  await run(
    SEARCHFOX_TOOL,
    [
      `search-files --limit=0 --include-dirs --group-by=directory --pathre '${pathre}' | batch-render dir`,
    ],
    {
      env: {
        ...process.env,
        SEARCHFOX_SERVER: treeDir("config.json"),
        SEARCHFOX_TREE: TREE,
      },
    }
  );
  return fs.readFileSync(treeDir("index/dir", dirPath, "index.html"));
}

async function render(requestPath) {
  await run(
    "cargo",
    [
      "build",
      "--release",
      "--bin",
      "output-file",
      "--bin",
      "crossref",
      "--bin",
      "searchfox-tool",
    ],
    { cwd: TOOLS }
  );
  const rev = await getRev();
  writeConfig();
  await fetchPerFileInfoInputs();
  await fetchWptManifests();
  const filePath = requestPath.replace(/\/$/, "");
  const isNew = !isDownloaded(filePath);
  if (
    requestPath.endsWith("/") ||
    !filePath ||
    !(await fetchFile(rev, filePath))
  ) {
    return renderDirectory(filePath);
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

  match = pathname.match(new RegExp(`^/${TREE}/source/(.*)$`));
  if (match && req.method == "GET") {
    const html = await queueRender(match[1]);
    if (html) {
      console.log(`Rendered ${match[1] || "/"}`);
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
