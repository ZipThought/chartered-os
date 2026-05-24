import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { createReadStream } from "node:fs";
import { mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dashboardDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(dashboardDir);
const workspaceRoot = path.resolve(process.env.WORKSPACE_ROOT ?? process.cwd());
const charteredDir = path.resolve(process.env.CHARTERED_DIR ?? path.join(workspaceRoot, ".chartered"));
const port = Number(process.env.PORT ?? 5177);

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://${req.headers.host}`);
    if (req.method === "GET" && url.pathname === "/workspace") {
      return json(res, await workspaceState());
    }
    if (req.method === "GET" && url.pathname === "/charter") {
      return json(res, await charterState());
    }
    if (req.method === "POST" && url.pathname === "/trigger/selection") {
      const body = await readJson(req);
      const triggerResult = await runSelection(body);
      return json(res, {
        workspace: await workspaceState(),
        run: triggerResult,
      });
    }
    if (req.method === "GET" && url.pathname.startsWith("/receipts/")) {
      return json(res, await receiptDetail(decodeURIComponent(url.pathname.slice(10))));
    }
    if (req.method === "GET" && url.pathname.startsWith("/findings/")) {
      return json(res, await findingDetail(decodeURIComponent(url.pathname.slice(10))));
    }
    if (req.method === "GET" && url.pathname.startsWith("/runs/")) {
      const tail = url.pathname.slice(6);
      if (tail.endsWith("/cognition")) {
        return json(res, await runCognition(tail.slice(0, -"/cognition".length)));
      }
    }
    return serveStatic(url.pathname, res);
  } catch (e) {
    res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
    res.end(e.stack ?? e.message);
  }
});

server.listen(port, () => {
  console.log(`Workspace console: http://127.0.0.1:${port}`);
  console.log(`WORKSPACE_ROOT=${workspaceRoot}`);
  console.log(`CHARTERED_DIR=${charteredDir}`);
});

async function workspaceState() {
  // Three independent disk walks; await in parallel.
  const [receipts, artifacts, findings] = await Promise.all([
    readReceipts(),
    listArtifacts(workspaceRoot),
    readFindings(),
  ]);
  return {
    workspace_root: workspaceRoot,
    chartered_dir: charteredDir,
    artifacts,
    findings,
    tasks: deriveTasks(receipts),
    attempts: deriveAttempts(receipts),
    receipts,
  };
}

async function charterState() {
  const { command, args } = runtimeCommand(["--print-charter"]);
  const result = await spawnCapture(command, args);
  if (result.code !== 0) {
    throw new Error(
      `chartered-runtime --print-charter exited ${result.code}: ${result.stderr || result.stdout}`,
    );
  }
  return JSON.parse(result.stdout);
}

async function listArtifacts(root) {
  const out = [];
  await visit(root, out);
  out.sort((a, b) => a.artifact_id.localeCompare(b.artifact_id));
  return out;
}

async function visit(dir, out) {
  if (path.basename(dir) === ".chartered") return;
  let entries = [];
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      await visit(full, out);
    } else if (isArtifactPath(full)) {
      const artifactId = path.relative(workspaceRoot, full).split(path.sep).join("/");
      out.push({
        artifact_id: artifactId,
        content: await readFile(full, "utf8"),
      });
    }
  }
}

function isArtifactPath(file) {
  return [".md", ".markdown", ".txt"].includes(path.extname(file));
}

async function readFindings() {
  // The kernel persists `kind=record-store` records to
  // `<chartered_dir>/<artifact_id>.jsonl`. The default record store
  // registers artifact_id `records`; the dashboard's "Findings" left-
  // rail node renders those records (which, in the demo, happen to
  // carry concern/severity/detail fields).
  const file = path.join(charteredDir, "records.jsonl");
  const text = await readOptional(file);
  return text
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

async function readReceipts() {
  const runs = path.join(charteredDir, "runs");
  let dirs = [];
  try {
    dirs = await readdir(runs);
  } catch {
    return [];
  }
  dirs.sort();
  // Read each run's receipts.jsonl in parallel; the final
  // timestamp-descending sort below restores the on-disk ordering
  // regardless of read completion order.
  const perRun = await Promise.all(
    dirs.map(async (run) => {
      const file = path.join(runs, run, "receipts.jsonl");
      const text = await readOptional(file);
      const out = [];
      for (const line of text.split("\n").filter(Boolean)) {
        const r = JSON.parse(line);
        r.run_id = run;
        out.push(r);
      }
      return out;
    }),
  );
  const receipts = perRun.flat();
  receipts.sort((a, b) => Number(b.timestamp ?? 0) - Number(a.timestamp ?? 0));
  return receipts;
}

function deriveTasks(receipts) {
  // Bucket receipts by task_id once, then derive each task's status
  // from its bucket. Was O(N²) on the receipts array via per-task
  // .filter; now O(N).
  const buckets = new Map();
  for (const r of receipts) {
    if (!r.task_id) continue;
    let bucket = buckets.get(r.task_id);
    if (!bucket) {
      bucket = { steward_id: r.steward_id, latest_timestamp: r.timestamp, items: [] };
      buckets.set(r.task_id, bucket);
    }
    bucket.items.push(r);
  }
  const tasks = [];
  for (const [task_id, bucket] of buckets) {
    tasks.push({
      task_id,
      steward_id: bucket.steward_id,
      status: taskStatusFor(bucket.items),
      latest_timestamp: bucket.latest_timestamp,
    });
  }
  tasks.sort((a, b) => Number(b.latest_timestamp ?? 0) - Number(a.latest_timestamp ?? 0));
  return tasks;
}

function deriveAttempts(receipts) {
  return receipts
    .filter((r) => r.attempt_id)
    .map((r) => ({
      attempt_id: r.attempt_id,
      task_id: r.task_id,
      steward_id: r.steward_id,
      receipt_id: r.receipt_id,
      outcome: r.outcome,
      tool: r.tool_call?.tool,
      timestamp: r.timestamp,
    }));
}

function taskStatusFor(receipts) {
  if (receipts.some((r) => r.tool_call?.tool === "<budget_exhausted>" || r.outcome === "Escalated")) return "Escalated";
  if (receipts.some((r) => r.tool_call?.tool === "<halt>")) return "Halted";
  if (receipts.some((r) => r.outcome === "Allowed" || r.outcome === "Passthrough")) return "Allowed";
  if (receipts.some((r) => r.outcome === "Denied")) return "Denied";
  return "Running";
}

async function receiptDetail(id) {
  const receipts = await readReceipts();
  const receipt = receipts.find((r) => r.receipt_id === id);
  if (!receipt) throw new Error(`receipt not found: ${id}`);
  const cognition = await runCognition(receipt.run_id);
  const task_receipts = receipts.filter((r) => r.task_id === receipt.task_id);
  return { receipt, cognition, task: deriveTasks(task_receipts)[0], attempts: deriveAttempts(task_receipts) };
}

async function findingDetail(id) {
  const finding = (await readFindings()).find((f) => f.id === id);
  if (!finding) throw new Error(`finding not found: ${id}`);
  return finding;
}

async function runCognition(runId) {
  if (!runId) return [];
  const file = path.join(charteredDir, "runs", runId, "cognition.jsonl");
  const text = await readOptional(file);
  return text
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

async function runSelection(body) {
  const artifactId = requireString(body, "artifact_id");
  const start = requireNumber(body, "start");
  const end = requireNumber(body, "end");
  const action = requireString(body, "action");
  const kind = requireString(body, "kind");
  const artifactPath = path.join(workspaceRoot, artifactId);
  const content = await readFile(artifactPath, "utf8");
  const startLine = lineForOffset(content, start);
  const endLine = lineForOffset(content, end);
  const { command, args } = runtimeCommand([
    "--chartered-dir",
    charteredDir,
    "--workspace-root",
    workspaceRoot,
    "--selection-artifact",
    artifactId,
    "--selection-start",
    String(start),
    "--selection-end",
    String(end),
    "--selection-start-line",
    String(startLine),
    "--selection-end-line",
    String(endLine),
    "--selection-action",
    action,
    "--selection-kind",
    kind,
  ]);
  const result = await spawnCapture(command, args);
  // `writeText` mkdirs its target directory, so no separate mkdir.
  await Promise.all([
    writeText(path.join(repoRoot, "temp", "dashboard-trigger.stdout.txt"), result.stdout),
    writeText(path.join(repoRoot, "temp", "dashboard-trigger.stderr.txt"), result.stderr),
  ]);
  if (result.code !== 0) {
    throw new Error(result.stderr || result.stdout || `runtime exited ${result.code}`);
  }
  let parsed = null;
  try {
    parsed = JSON.parse(result.stdout);
  } catch {
    parsed = null;
  }
  return { stdout: parsed, exit: result.code };
}

function runtimeCommand(runtimeArgs) {
  if (process.env.CHARTERED_RUNTIME_BIN) {
    return { command: process.env.CHARTERED_RUNTIME_BIN, args: runtimeArgs };
  }
  return {
    command: "cargo",
    args: [
      "run",
      "--quiet",
      "--manifest-path",
      path.join(repoRoot, "runtime", "Cargo.toml"),
      "--bin",
      "chartered-runtime",
      "--",
      ...runtimeArgs,
    ],
  };
}

function spawnCapture(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, env: process.env });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (buf) => (stdout += buf));
    child.stderr.on("data", (buf) => (stderr += buf));
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

function lineForOffset(content, offset) {
  return content.slice(0, offset).split("\n").length;
}

function requireString(body, name) {
  const value = body[name];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`missing ${name}`);
  }
  return value;
}

function requireNumber(body, name) {
  const value = body[name];
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`missing ${name}`);
  }
  return value;
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("error", reject);
    req.on("end", () => {
      try {
        resolve(JSON.parse(body || "{}"));
      } catch (e) {
        reject(e);
      }
    });
  });
}

async function readOptional(file) {
  try {
    return await readFile(file, "utf8");
  } catch {
    return "";
  }
}

async function writeText(file, text) {
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, text);
}

async function serveStatic(requestPath, res) {
  const clean = requestPath === "/" ? "/index.html" : requestPath;
  const file = path.resolve(dashboardDir, `.${clean}`);
  if (!file.startsWith(dashboardDir)) {
    res.writeHead(403);
    res.end("forbidden");
    return;
  }
  try {
    const info = await stat(file);
    if (!info.isFile()) throw new Error("not a file");
  } catch {
    res.writeHead(404);
    res.end("not found");
    return;
  }
  res.writeHead(200, { "content-type": contentType(file) });
  createReadStream(file).pipe(res);
}

function contentType(file) {
  switch (path.extname(file)) {
    case ".html":
      return "text/html; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".js":
    case ".mjs":
      return "text/javascript; charset=utf-8";
    default:
      return "application/octet-stream";
  }
}

function json(res, value) {
  res.writeHead(200, { "content-type": "application/json; charset=utf-8" });
  res.end(JSON.stringify(value));
}
