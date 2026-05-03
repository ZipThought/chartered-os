// ─────────────────────────────────────────────
// CharteredOS Workspace Console — SolidJS rebuild
//
// No build step. ESM imports load SolidJS from esm.sh. State lives in
// signals/resources/stores; the DOM is purely an output of that state.
// Event handlers bind directly via `onClick=${handler}` closures —
// no JSON-in-attribute, no `JSON.parse(dataset.x)` indirection.
//
// CDN URLs (pinned to a specific minor for reproducibility):
//   solid-js@1.8 — primitives (createSignal, createResource, For, Show, …)
//   solid-js/web — `render` for mounting
//   solid-js/store — fine-grained nested state
//   solid-js/html — html`…` template-literal templates
// ─────────────────────────────────────────────

import { render } from "https://esm.sh/solid-js@1.8.22/web";
import {
  createSignal,
  createResource,
  createMemo,
  createEffect,
  For,
  Show,
  Switch,
  Match,
} from "https://esm.sh/solid-js@1.8.22";
import { createStore } from "https://esm.sh/solid-js@1.8.22/store";
import html from "https://esm.sh/solid-js@1.8.22/html";

// ─────────────────────────────────────────────
// Constants — colors and labels mirror the kernel's Verdict / Outcome enums
// ─────────────────────────────────────────────

const RULING = {
  Grounded: { fg: "var(--grn)", bg: "var(--grn-bg)" },
  Ungrounded: { fg: "var(--red)", bg: "var(--red-bg)" },
  Uncertain: { fg: "var(--amb)", bg: "var(--amb-bg)" },
  OutOfScope: { fg: "var(--vio)", bg: "var(--vio-bg)" },
};

const OUTCOME = {
  Allowed: { fg: "var(--grn)", bg: "var(--grn-bg)" },
  Denied: { fg: "var(--red)", bg: "var(--red-bg)" },
  Escalated: { fg: "var(--amb)", bg: "var(--amb-bg)" },
  Passthrough: { fg: "var(--vio)", bg: "var(--vio-bg)" },
};

const SEV = {
  high: "var(--red)",
  medium: "var(--amb)",
  low: "var(--t3)",
};

// Per spec §Receipts: the trail is "the append-only record of one Gate
// step." Halts and fails are control-flow sentinels, not Gate steps —
// they're filtered from the user-facing Receipts trail.
function isLoopSentinel(toolName) {
  return toolName === "<halt>" || toolName === "<fail>";
}

// ─────────────────────────────────────────────
// API
// ─────────────────────────────────────────────

async function api(path, opts) {
  const res = await fetch(path, opts);
  if (!res.ok) {
    throw new Error(`${path}: ${res.status} ${await res.text()}`);
  }
  return res.json();
}

const fetchWorkspace = () => api("/workspace");
const fetchCharter = () => api("/charter");
const fetchReceiptDetail = (id) =>
  api(`/receipts/${encodeURIComponent(id)}`);
const fetchFindingDetail = (id) =>
  api(`/findings/${encodeURIComponent(id)}`);

async function postSelection(body) {
  return api("/trigger/selection", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

// ─────────────────────────────────────────────
// Helpers: format, markdown
// ─────────────────────────────────────────────

function tsLabel(ts_ns) {
  if (!ts_ns) return "";
  const ms = Number(ts_ns) / 1e6;
  return new Date(ms).toLocaleTimeString();
}

function fieldString(v) {
  if (v == null) return "";
  if (typeof v === "string") return v;
  if (typeof v === "object" && "value" in v) return String(v.value);
  return String(v);
}

function frameRefId(v) {
  if (!v) return "";
  if (v.frame_ref) return `${fieldString(v.frame_ref.steward_id)} / ${fieldString(v.frame_ref.frame_id)}`;
  if (v.steward_id && v.frame_id) return `${fieldString(v.steward_id)} / ${fieldString(v.frame_id)}`;
  return fieldString(v.frame_id ?? v);
}

function escapeHtml(value) {
  // Inner-text escaping only. Used for the markdown preview's
  // dangerouslySet content. Click dispatch never depends on HTML
  // attribute escaping — handlers are JS closures.
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

// Minimal markdown → HTML for the Preview tab. Headings, paragraphs,
// bold/italic, inline code, fenced code blocks, lists, blockquotes.
function renderMarkdown(src) {
  const escaped = escapeHtml(src);
  const lines = escaped.split("\n");
  const out = [];
  let inCode = false;
  let inList = false;
  let inQuote = false;
  let para = [];
  const flushPara = () => {
    if (para.length) {
      out.push(`<p>${inlineMd(para.join(" "))}</p>`);
      para = [];
    }
  };
  const flushList = () => {
    if (inList) {
      out.push("</ul>");
      inList = false;
    }
  };
  const flushQuote = () => {
    if (inQuote) {
      out.push("</blockquote>");
      inQuote = false;
    }
  };
  for (const raw of lines) {
    if (raw.match(/^```/)) {
      flushPara();
      flushList();
      flushQuote();
      inCode = !inCode;
      out.push(inCode ? "<pre><code>" : "</code></pre>");
      continue;
    }
    if (inCode) {
      out.push(raw);
      continue;
    }
    const h1 = raw.match(/^# (.+)$/);
    const h2 = raw.match(/^## (.+)$/);
    const h3 = raw.match(/^### (.+)$/);
    const li = raw.match(/^[-*] (.+)$/);
    const ol = raw.match(/^\d+\. (.+)$/);
    const bq = raw.match(/^> (.+)$/);
    if (h1) {
      flushPara();
      flushList();
      flushQuote();
      out.push(`<h1>${inlineMd(h1[1])}</h1>`);
    } else if (h2) {
      flushPara();
      flushList();
      flushQuote();
      out.push(`<h2>${inlineMd(h2[1])}</h2>`);
    } else if (h3) {
      flushPara();
      flushList();
      flushQuote();
      out.push(`<h3>${inlineMd(h3[1])}</h3>`);
    } else if (li || ol) {
      flushPara();
      flushQuote();
      if (!inList) {
        out.push("<ul>");
        inList = true;
      }
      out.push(`<li>${inlineMd((li ?? ol)[1])}</li>`);
    } else if (bq) {
      flushPara();
      flushList();
      if (!inQuote) {
        out.push("<blockquote>");
        inQuote = true;
      }
      out.push(`<p>${inlineMd(bq[1])}</p>`);
    } else if (raw.trim() === "") {
      flushPara();
      flushList();
      flushQuote();
    } else {
      para.push(raw.trim());
    }
  }
  flushPara();
  flushList();
  flushQuote();
  return out.join("\n");
}

function inlineMd(s) {
  return s
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(?<!\*)\*([^*]+)\*(?!\*)/g, "<em>$1</em>");
}

// ─────────────────────────────────────────────
// Top-level state
// ─────────────────────────────────────────────

const [workspace, { refetch: refetchWorkspace }] =
  createResource(fetchWorkspace);
const [charter, { refetch: refetchCharter }] = createResource(fetchCharter);

const [selectedNode, setSelectedNode] = createSignal({ type: "welcome" });
const [activeTab, setActiveTab] = createSignal("edit"); // edit | preview | source | parsed
const [editorSelection, setEditorSelection] = createSignal(null); // { artifact_id, start, end, text }
const [processing, setProcessing] = createSignal(null); // { label, started, artifact_id }
const [banners, setBanners] = createSignal([]);
const [expanded, setExpanded] = createStore({
  ch: true, // Charter category
  art: true, // Artifacts category
  rec: true, // Receipts category
  fn: true, // Findings category
  fr: false, // Frames sub
  rv: false, // Reviewers sub
  ac: false, // Actions sub
});

// Per-receipt and per-finding detail caches keyed by id; populated on
// first navigation, refetched on demand.
const [receiptCache, setReceiptCache] = createStore({});
const [findingCache, setFindingCache] = createStore({});

// Per-artifact local edit buffer (textarea content). Kept distinct
// from the workspace fetch so user edits don't get clobbered by
// background refetches.
const [buffers, setBuffers] = createStore({});

let bannerSeq = 0;

function pushBanner(b) {
  const id = ++bannerSeq;
  const banner = { ...b, id };
  setBanners((bs) => [...bs, banner]);
  if (b.kind !== "danger") {
    setTimeout(() => closeBanner(id), 9000);
  }
  return id;
}

function closeBanner(id) {
  setBanners((bs) => bs.filter((b) => b.id !== id));
}

// ─────────────────────────────────────────────
// Derived state
// ─────────────────────────────────────────────

const visibleReceipts = createMemo(() => {
  const ws = workspace();
  if (!ws) return [];
  return (ws.receipts ?? []).filter(
    (r) => !isLoopSentinel(r.tool_call?.tool),
  );
});

const visibleFindings = createMemo(() => workspace()?.findings ?? []);
const visibleTasks = createMemo(() => workspace()?.tasks ?? []);

const modeLabel = createMemo(() => {
  const m = charter()?.governance_mode;
  if (!m) return "—";
  if (m.grounding && m.evaluation) return "full";
  if (!m.grounding && !m.evaluation) return "neither";
  if (m.grounding) return "grounding-only";
  return "evaluation-only";
});

// ─────────────────────────────────────────────
// Mutations
// ─────────────────────────────────────────────

function navigate(target) {
  setSelectedNode(target);
  setActiveTab(target.type === "charter" ? "parsed" : "edit");
  if (target.type !== "artifact") {
    setEditorSelection(null);
  }
  // Lazy-fetch receipt detail + cognition trace
  if (target.type === "receipt" && !receiptCache[target.id]) {
    setReceiptCache(target.id, { loading: true });
    fetchReceiptDetail(target.id)
      .then((d) =>
        setReceiptCache(target.id, { ...d, loading: false }),
      )
      .catch((e) =>
        setReceiptCache(target.id, {
          loading: false,
          error: e.message,
        }),
      );
  }
  // Lazy-fetch finding detail
  if (target.type === "finding" && !findingCache[target.id]) {
    setFindingCache(target.id, { loading: true });
    fetchFindingDetail(target.id)
      .then((d) =>
        setFindingCache(target.id, { value: d, loading: false }),
      )
      .catch((e) =>
        setFindingCache(target.id, {
          loading: false,
          error: e.message,
        }),
      );
  }
}

function toggleExpand(key) {
  setExpanded(key, (v) => !v);
}

async function triggerAction(action) {
  const sel = editorSelection();
  if (!sel) return;
  if (processing()) return;
  setProcessing({
    label: `${action.name}…`,
    started: Date.now(),
    artifact_id: sel.artifact_id,
  });
  try {
    const result = await postSelection({
      artifact_id: sel.artifact_id,
      start: sel.start,
      end: sel.end,
      action: action.name,
      kind: action.kind === "evaluative" ? "evaluative" : "generative",
    });
    // Re-pull workspace for the latest receipts/findings
    await refetchWorkspace();
    // Charter usually doesn't change per-trigger; refetch defensively
    refetchCharter();
    summarizeRunInBanner(action, result.run);
  } catch (e) {
    pushBanner({ kind: "danger", text: `${action.name} failed: ${e.message}` });
  } finally {
    setProcessing(null);
  }
}

function summarizeRunInBanner(action, run) {
  const stdout = run?.stdout;
  if (!stdout) {
    pushBanner({
      kind: "warn",
      text: `${action.name}: runtime returned no parseable response`,
    });
    return;
  }
  const receipts = (stdout.receipts ?? []).filter(
    (r) => !isLoopSentinel(r.tool_call?.tool),
  );
  if (receipts.length === 0) {
    pushBanner({
      kind: "warn",
      text: `${action.name}: no Gate-evaluable Tool call produced (Steward halted)`,
    });
    return;
  }
  const last = receipts[receipts.length - 1];
  const outcome = last.outcome;
  const ungrounded = (last.verdicts ?? []).filter(
    (v) => v.ruling === "Ungrounded",
  );
  let text;
  let kind;
  if (outcome === "Allowed" || outcome === "Passthrough") {
    text = `${action.name} → ${outcome}`;
    kind = "ok";
  } else {
    const reasons =
      ungrounded.length > 0
        ? ungrounded
            .map((v) => `${frameRefId(v)}: ${v.reason}`)
            .join(" · ")
        : "no specific frame reason";
    text = `${action.name} → ${outcome} — ${reasons}`;
    kind = outcome === "Denied" ? "warn" : "danger";
  }
  pushBanner({ kind, text, receipt_id: last.receipt_id });
}

// ─────────────────────────────────────────────
// Components
// ─────────────────────────────────────────────

function App() {
  return html`
    <div class="R">
      <${Header}/>
      <div class="main">
        <aside class="pnl tree-pnl">
          <${LeftRail}/>
        </aside>
        <${Resizer} side="L"/>
        <main class="pnl center-pnl">
          <${CenterPane}/>
        </main>
        <${Resizer} side="R"/>
        <aside class="pnl right-pnl">
          <${RightRail}/>
        </aside>
      </div>
      <${BannerStack}/>
    </div>
  `;
}

function Header() {
  return html`
    <header class="hdr">
      <h1><span>Chartered</span>OS Workspace</h1>
      <div class="hdr-r">
        <span>Charter v${() => charter()?.charter_ref?.version ?? "—"}</span>
        <span>${() => workspace()?.artifacts?.length ?? 0} artifacts</span>
        <span>${() => visibleReceipts().length} receipts</span>
        <span class="badge mode">mode: ${() => modeLabel()}</span>
      </div>
    </header>
  `;
}

// ─────────────────────────────────────────────
// Left rail: tree
// ─────────────────────────────────────────────

function LeftRail() {
  return html`
    <div class="ph">
      <span>Workspace</span>
      <button class="ibtn"
              onClick=${() => { refetchWorkspace(); refetchCharter(); }}
              title="Refresh"
      >↻</button>
    </div>
    <div class="pb">
      <${TreeCategory} key="ch" label="Charter">
        <${CharterSubtree}/>
      <//>
      <${TreeCategory} key="art" label="Artifacts" count=${() => workspace()?.artifacts?.length ?? 0}>
        <${For} each=${() => workspace()?.artifacts ?? []}>
          ${(a) => html`
            <${TreeLeaf}
              target=${{ type: "artifact", id: a.artifact_id }}
              label=${a.artifact_id}
              icon="◈"
            />
          `}
        <//>
      <//>
      <${TreeCategory} key="tsk" label="Tasks" count=${() => visibleTasks().length}>
        <${For} each=${visibleTasks}>
          ${(t) => html`
            <${TreeLeaf}
              target=${{ type: "task", id: t.task_id }}
              label=${`${String(t.task_id).slice(0, 18)} · ${t.status}`}
              icon=${t.status === "Allowed" || t.status === "Halted" ? "✓" : t.status === "Denied" ? "✗" : "!"}
            />
          `}
        <//>
      <//>
      <${TreeCategory} key="rec" label="Receipts" count=${() => visibleReceipts().length}>
        <${For} each=${visibleReceipts}>
          ${(r) => html`
            <${TreeLeaf}
              target=${{ type: "receipt", id: r.receipt_id }}
              label=${`${r.tool_call?.tool ?? "—"} · ${r.outcome}`}
              icon=${r.outcome === "Allowed" ? "✓" : r.outcome === "Denied" ? "✗" : "!"}
            />
          `}
        <//>
      <//>
      <${TreeCategory} key="fn" label="Findings" count=${() => visibleFindings().length}>
        <${For} each=${visibleFindings}>
          ${(f) => html`
            <${TreeLeaf}
              target=${{ type: "finding", id: f.id }}
              label=${`${f.severity} · ${f.concern}`}
              icon="◆"
            />
          `}
        <//>
      <//>
    </div>
  `;
}

function CharterSubtree() {
  return html`
    <${TreeLeaf}
      target=${{ type: "charter", id: "behavioral" }}
      label="Behavioral spec"
      icon="¶"
    />
    <${TreeLeaf}
      target=${{ type: "charter", id: "scopes" }}
      label="Scopes"
      icon="§"
    />
    <${TreeSubcategory} key="st" label="Stewards" count=${() => charter()?.stewards?.length ?? 0}>
      <${For} each=${() => charter()?.stewards ?? []}>
        ${(s) => html`
          <${TreeLeaf}
            target=${{ type: "reviewer", id: s.id }}
            label=${s.display_name ?? s.id}
            icon="◎"
            indent=${2}
          />
          <${For} each=${() => s.frames ?? []}>
            ${(f) => html`
              <${TreeLeaf}
                target=${{ type: "frame", id: `${s.id}/${f.id}` }}
                label=${f.id}
                icon="◇"
                indent=${3}
              />
            `}
          <//>
        `}
      <//>
    <//>
    <${TreeSubcategory} key="ac" label="Actions" count=${() => charter()?.actions?.length ?? 0}>
      <${For} each=${() => charter()?.actions ?? []}>
        ${(a) => html`
          <${TreeLeaf}
            target=${{ type: "action", id: a.name }}
            label=${`${a.name} · ${a.kind}`}
            icon=${a.kind === "generative" ? "⊕" : "⊙"}
            indent=${2}
          />
        `}
      <//>
    <//>
  `;
}

function TreeCategory(props) {
  const open = () => expanded[props.key];
  return html`
    <div class="tn cat" onClick=${() => toggleExpand(props.key)}>
      <span class="tn-chev">${() => (open() ? "▾" : "▸")}</span>
      <span class="tn-label">${props.label}</span>
      <${Show} when=${() => props.count != null}>
        <span class="tn-cnt">${props.count}</span>
      <//>
    </div>
    <${Show} when=${open}>
      ${props.children}
    <//>
  `;
}

function TreeSubcategory(props) {
  const open = () => expanded[props.key];
  return html`
    <div class="tn cat" style="padding-left:14px;" onClick=${() => toggleExpand(props.key)}>
      <span class="tn-chev">${() => (open() ? "▾" : "▸")}</span>
      <span class="tn-label">${props.label}</span>
      <${Show} when=${() => props.count != null}>
        <span class="tn-cnt">${props.count}</span>
      <//>
    </div>
    <${Show} when=${open}>
      ${props.children}
    <//>
  `;
}

function TreeLeaf(props) {
  const isOn = createMemo(() => sameTarget(selectedNode(), props.target));
  return html`
    <div class=${() => `tn tn-leaf ${isOn() ? "on" : ""}`}
         style=${`padding-left:${(props.indent ?? 1) * 14 + 4}px;`}
         onClick=${() => navigate(props.target)}
         title=${props.label}
    >
      <span class="tn-icon">${props.icon ?? "·"}</span>
      <span class="tn-label">${props.label}</span>
    </div>
  `;
}

function sameTarget(a, b) {
  return a && b && a.type === b.type && a.id === b.id;
}

// ─────────────────────────────────────────────
// Center pane
// ─────────────────────────────────────────────

function CenterPane() {
  return html`
    <${Switch} fallback=${html`<${WelcomePage}/>`}>
      <${Match} when=${() => selectedNode().type === "artifact"}>
        <${ArtifactPage} aid=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "charter"}>
        <${CharterPage} sub=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "receipt"}>
        <${ReceiptPage} id=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "task"}>
        <${TaskPage} id=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "frame"}>
        <${FramePage} id=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "reviewer"}>
        <${ReviewerPage} id=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "action"}>
        <${ActionPage} id=${() => selectedNode().id}/>
      <//>
      <${Match} when=${() => selectedNode().type === "finding"}>
        <${FindingPage} id=${() => selectedNode().id}/>
      <//>
    <//>
  `;
}

function WelcomePage() {
  return html`
    <div class="welcome">
      <h1>CharteredOS Workspace Console</h1>
      <p>
        Select an artifact, frame, action, or receipt from the left rail.
        Open a text artifact, highlight a span, then trigger one of the
        Charter's Actions to see the negative-feedback loop run end-to-end:
        the Steward proposes a Tool call, the Gate evaluates it against
        the active Frames, and a Receipt is appended.
      </p>
      <p>
        Halts and fails are filtered from the Receipts trail per spec
        §Receipts.
      </p>
    </div>
  `;
}

// ─────────────────────────────────────────────
// Artifact page: tabs (edit / preview), action bar, status line
// ─────────────────────────────────────────────

function ArtifactPage(props) {
  const artifact = createMemo(() =>
    (workspace()?.artifacts ?? []).find(
      (a) => a.artifact_id === props.aid,
    ),
  );

  // Hydrate buffer on first observation of this artifact_id, and on
  // workspace refetches that bring in updated content from the runtime
  // (e.g. after an Allowed Refine).
  createEffect(() => {
    const a = artifact();
    if (!a) return;
    setBuffers(a.artifact_id, a.content);
  });

  // Reset selection whenever the active artifact changes.
  createEffect(() => {
    const id = props.aid;
    const sel = editorSelection();
    if (sel && sel.artifact_id !== id) setEditorSelection(null);
  });

  return html`
    <${Show}
      when=${artifact}
      fallback=${html`<div class="welcome"><p>Artifact not found.</p></div>`}
    >
      <div class="tabs">
        <div class=${() => `tab ${activeTab() === "edit" ? "on" : ""}`}
             onClick=${() => setActiveTab("edit")}>Edit</div>
        <div class=${() => `tab ${activeTab() === "preview" ? "on" : ""}`}
             onClick=${() => setActiveTab("preview")}>Preview</div>
        <div class="tab-meta">
          <span>${() => artifact()?.artifact_id}</span>
        </div>
      </div>
      <${Switch}>
        <${Match} when=${() => activeTab() === "edit"}>
          <${ArtifactEditor} artifact=${artifact}/>
        <//>
        <${Match} when=${() => activeTab() === "preview"}>
          <${ArtifactPreview} artifact=${artifact}/>
        <//>
      <//>
      <${ActionBar}/>
      <${StatusLine}/>
    <//>
  `;
}

function ArtifactEditor(props) {
  let taRef;
  const content = () => buffers[props.artifact?.artifact_id ?? ""] ?? "";

  const captureSelection = () => {
    if (!taRef) return;
    const start = taRef.selectionStart;
    const end = taRef.selectionEnd;
    const id = props.artifact?.artifact_id;
    if (!id || start === end) {
      setEditorSelection(null);
      return;
    }
    const text = content().slice(start, end);
    setEditorSelection({ artifact_id: id, start, end, text });
  };

  return html`
    <div class="ew">
      <textarea
        class="ta"
        ref=${(el) => (taRef = el)}
        value=${content}
        onInput=${(e) => setBuffers(props.artifact.artifact_id, e.target.value)}
        onSelect=${captureSelection}
        onKeyUp=${captureSelection}
        onMouseUp=${captureSelection}
        spellcheck="false"
      ></textarea>
    </div>
  `;
}

function ArtifactPreview(props) {
  const md = createMemo(() => renderMarkdown(buffers[props.artifact?.artifact_id ?? ""] ?? ""));
  return html`<div class="prev" innerHTML=${md}></div>`;
}

function ActionBar() {
  const actions = () => charter()?.actions ?? [];
  return html`
    <div class="ab">
      <${Show}
        when=${editorSelection}
        fallback=${html`<span class="abnote">Select text in the artifact above to enable an Action.</span>`}
      >
        <span class="abl">
          ${() => editorSelection().artifact_id} · ${() => editorSelection().start}–${() => editorSelection().end}
        </span>
        <${For} each=${actions}>
          ${(a) => html`
            <button class=${() => `abtn ${a.kind === "evaluative" ? "eval" : ""}`}
                    onClick=${() => triggerAction(a)}
                    disabled=${() => !!processing()}
                    title=${a.prompt ?? a.name}>
              ${a.name}
            </button>
          `}
        <//>
        <${Show} when=${processing}>
          <span class="abl" style="margin-left:auto;">
            <span class="dot" style="background:var(--acc);display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:6px;"></span>
            ${() => processing().label}
          </span>
        <//>
      <//>
    </div>
  `;
}

function StatusLine() {
  return html`
    <div class="sts">
      <${Show}
        when=${editorSelection}
        fallback=${html`<span style="color:var(--t3);">no selection</span>`}
      >
        <span style="color:var(--t2);">
          ${() => editorSelection().text.length} chars selected ·
          ${() => editorSelection().text.split("\n").length} line(s)
        </span>
      <//>
    </div>
  `;
}

// ─────────────────────────────────────────────
// Charter page
// ─────────────────────────────────────────────

function CharterPage(props) {
  return html`
    <div class="tabs">
      <div class=${() => `tab ${activeTab() === "parsed" ? "on" : ""}`}
           onClick=${() => setActiveTab("parsed")}>Parsed</div>
      <div class=${() => `tab ${activeTab() === "source" ? "on" : ""}`}
           onClick=${() => setActiveTab("source")}>Source</div>
      <div class="tab-meta">
        <span>Charter v${() => charter()?.charter_ref?.version ?? "—"}</span>
      </div>
    </div>
    <${Switch}>
      <${Match} when=${() => activeTab() === "parsed"}>
        <${CharterParsed} sub=${props.sub}/>
      <//>
      <${Match} when=${() => activeTab() === "source"}>
        <${CharterSource}/>
      <//>
    <//>
  `;
}

function CharterParsed(props) {
  return html`
    <div class="det">
      <${Switch}
        fallback=${html`<p>Pick a Charter sub-node from the left rail.</p>`}
      >
        <${Match} when=${() => props.sub === "behavioral"}>
          <h1>Behavioral spec</h1>
          <pre>${() => charter()?.behavioral_spec ?? ""}</pre>
        <//>
        <${Match} when=${() => props.sub === "scopes"}>
          <h1>Charter Scopes</h1>
          <${For} each=${() => charter()?.scopes ?? []}>
            ${(s) => html`
              <div class="scope-block charter">
                <div class="nm">${s.name}</div>
                <div class="bd">${s.text}</div>
              </div>
            `}
          <//>
        <//>
      <//>
    </div>
  `;
}

function CharterSource() {
  return html`
    <div class="det">
      <h1>Source</h1>
      <p style="font-size:11px;color:var(--t3);">
        Read-only. Charter editing arrives via the Charter Editor Foundation Steward
        (spec §The Charter).
      </p>
      <h2>Behavioral spec</h2>
      <pre>${() => charter()?.behavioral_spec ?? ""}</pre>
      <h2>Frames</h2>
      <pre>${() => JSON.stringify(charter()?.frames ?? [], null, 2)}</pre>
      <h2>Reviewers</h2>
      <pre>${() => JSON.stringify(charter()?.reviewers ?? [], null, 2)}</pre>
      <h2>Actions</h2>
      <pre>${() => JSON.stringify(charter()?.actions ?? [], null, 2)}</pre>
    </div>
  `;
}

// ─────────────────────────────────────────────
// Receipt detail page
// ─────────────────────────────────────────────

function ReceiptPage(props) {
  const cached = createMemo(() => receiptCache[props.id]);
  return html`
    <div class="det">
      <${Show}
        when=${() => cached()?.receipt}
        fallback=${html`<p>${() => cached()?.error ?? "Loading…"}</p>`}
      >
        ${() => {
          const r = cached().receipt;
          const cog = cached().cognition ?? [];
          return html`
            <h1>Receipt
              <span style="font-family:var(--mono);font-size:13px;color:var(--t3);">
                ${r.receipt_id}
              </span>
            </h1>
            <div class="meta">
              <span>steward · ${r.steward_id}</span>
              <span>task · ${r.task_id}</span>
              <span>attempt · ${r.attempt_id ?? "controller"}</span>
              <span>tool · ${r.tool_call?.tool}</span>
              <span style=${`color:${OUTCOME[r.outcome]?.fg};font-weight:600;`}>${r.outcome}</span>
              <span>charter · v${r.charter_version}</span>
              <span>role_ctx · v${r.role_context_version}</span>
              <span>snapshot · ${String(r.snapshot_id).slice(0, 12)}…</span>
              <${Show} when=${!r.intercept_complete}>
                <span style="color:var(--amb);">intercept incomplete</span>
              <//>
            </div>

            <h2>Verdicts</h2>
            <div>
              <${For} each=${r.verdicts ?? []}>
                ${(v) => html`
                  <div class="ruling-row">
                    <span class="b badge"
                          style=${`color:${RULING[v.ruling]?.fg ?? "var(--t2)"};background:${RULING[v.ruling]?.bg ?? "var(--bg-card-alt)"};`}>
                      ${v.ruling}
                    </span>
                    <span class="nm" onClick=${() => navigate({ type: "frame", id: `${fieldString(v.frame_ref?.steward_id)}/${fieldString(v.frame_ref?.frame_id)}` })}>
                      ${frameRefId(v)}
                    </span>
                    <span class="rs">${v.reason || "—"}</span>
                  </div>
                `}
              <//>
            </div>

            <div class="struct-sep">
              Structural Separation invariant — the Evaluator received only the
              proposed Tool call and the Frame's declared Scope content, never
              the Actor's conversation history or reasoning. Spec §Structural Separation.
            </div>

            <h2>Tool call</h2>
            <pre>${JSON.stringify(r.tool_call, null, 2)}</pre>

            <h2>Cognition / Evaluator prompts</h2>
            <div class="dual">
              <div>
                <h3>Cognition (Actor)</h3>
                <${CognitionPanes} entries=${cog} role="actor"/>
              </div>
              <div>
                <h3>Evaluator</h3>
                <${CognitionPanes} entries=${cog} role="eval"/>
              </div>
            </div>
          `;
        }}
      <//>
    </div>
  `;
}

function CognitionPanes(props) {
  const filtered = () =>
    (props.entries ?? []).filter((c) =>
      props.role === "actor"
        ? c.backend_id === "actor"
        : c.backend_id?.startsWith("eval"),
    );
  return html`
    <${Show} when=${() => filtered().length > 0} fallback=${html`<pre>(none)</pre>`}>
      <${For} each=${filtered}>
        ${(e, i) => html`
          <pre style="margin-bottom:8px;">${
            `[${i() + 1}] ${e.backend_id}\n` +
            (e.request?.messages ?? [])
              .map((m) => `--- ${m.role} ---\n${m.content}`)
              .join("\n\n") +
            `\n--- response ---\n${e.response?.text ?? ""}`
          }</pre>
        `}
      <//>
    <//>
  `;
}

// ─────────────────────────────────────────────
// Frame / Reviewer / Action / Finding pages
// ─────────────────────────────────────────────

function splitFrameNodeId(id) {
  const [stewardId, frameId] = String(id).split("/", 2);
  return { stewardId, frameId: frameId ?? stewardId };
}

function FramePage(props) {
  const frame = createMemo(() => {
    const { stewardId, frameId } = splitFrameNodeId(props.id);
    const steward = (charter()?.stewards ?? []).find((s) => s.id === stewardId);
    return {
      steward,
      frame: (steward?.frames ?? charter()?.frames ?? []).find((x) => x.id === frameId),
    };
  });
  return html`
    <div class="det">
      <${Show} when=${() => frame().frame} fallback=${html`<p>Frame not found.</p>`}>
        <h1>Frame · ${() => frame().frame.id}</h1>
        <div class="meta"><span>steward · ${() => frame().steward?.id ?? "sut"}</span></div>
        <div class="summary">${() => frame().frame.concern}</div>
        <h2>Applies to tools</h2>
        <p>${() => (frame().frame.applies_to_tools ?? []).join(", ") || "—"}</p>
        <h2>Declared scopes</h2>
        <p>${() => (frame().frame.declared_scopes ?? []).map((s) => `${s.kind}:${s.name}`).join(", ") || "—"}</p>
      <//>
    </div>
  `;
}

function ReviewerPage(props) {
  const r = createMemo(() => (charter()?.stewards ?? []).find((x) => x.id === props.id));
  return html`
    <div class="det">
      <${Show} when=${r} fallback=${html`<p>Steward not found.</p>`}>
        <h1>Steward · ${() => r().display_name ?? r().id}</h1>
        <div class="meta"><span>role · ${() => r().role}</span><span>id · ${() => r().id}</span></div>
        <h2>Frames</h2>
        <pre>${() => JSON.stringify(r().frames ?? [], null, 2)}</pre>
      <//>
    </div>
  `;
}

function TaskPage(props) {
  const task = createMemo(() => visibleTasks().find((x) => x.task_id === props.id));
  const receipts = createMemo(() => (workspace()?.receipts ?? []).filter((r) => r.task_id === props.id));
  return html`
    <div class="det">
      <${Show} when=${task} fallback=${html`<p>Task not found.</p>`}>
        <h1>Task · ${() => task().task_id}</h1>
        <div class="meta">
          <span>steward · ${() => task().steward_id}</span>
          <span>status · ${() => task().status}</span>
        </div>
        <h2>Receipts</h2>
        <div>
          <${For} each=${receipts}>
            ${(r) => html`
              <div class="ctx-item" onClick=${() => navigate({ type: "receipt", id: r.receipt_id })}>
                <div class="ctx-h">
                  <span class="ctx-t">${r.tool_call?.tool}</span>
                  <span class="badge" style=${`color:${OUTCOME[r.outcome]?.fg};background:${OUTCOME[r.outcome]?.bg};`}>${r.outcome}</span>
                </div>
                <div class="ctx-meta">attempt · ${r.attempt_id ?? "controller"} · ${String(r.receipt_id).slice(0, 10)}…</div>
              </div>
            `}
          <//>
        </div>
      <//>
    </div>
  `;
}

function ActionPage(props) {
  const a = createMemo(() => (charter()?.actions ?? []).find((x) => x.name === props.id));
  return html`
    <div class="det">
      <${Show} when=${a} fallback=${html`<p>Action not found.</p>`}>
        <h1>Action · ${() => a().name}</h1>
        <div class="meta">
          <span>kind · ${() => a().kind}</span>
        </div>
        <h2>Prompt <span class="det-sub">(injected into the trigger message)</span></h2>
        <pre>${() => a().prompt ?? ""}</pre>
      <//>
    </div>
  `;
}

function FindingPage(props) {
  const f = createMemo(() => (visibleFindings() ?? []).find((x) => x.id === props.id));
  return html`
    <div class="det">
      <${Show} when=${f} fallback=${html`<p>Finding not found.</p>`}>
        <h1>Finding · ${() => f().id}</h1>
        <div class="meta">
          <span>severity · <span style=${() => `color:${SEV[f().severity] ?? "var(--t2)"};font-weight:600;`}>${() => f().severity}</span></span>
          <span>task · ${() => fieldString(f().task_id)}</span>
          <span>steward · ${() => fieldString(f().author_steward_id)}</span>
          <span>artifact · ${() => fieldString(f().artifact_id)}</span>
        </div>
        <div class="summary">${() => f().concern}</div>
        <h2>Detail</h2>
        <pre>${() => f().detail ?? ""}</pre>
        <h2>From Receipt</h2>
        <p>
          <span class="ctx-item" style="display:inline-block;cursor:pointer;"
                onClick=${() => navigate({ type: "receipt", id: fieldString(f().admitting_receipt_id) })}>
            ${() => fieldString(f().admitting_receipt_id)}
          </span>
        </p>
      <//>
    </div>
  `;
}

// ─────────────────────────────────────────────
// Right rail
// ─────────────────────────────────────────────

function RightRail() {
  return html`
    <div class="ph"><span>Context</span></div>
    <div class="pb">
      <${Switch} fallback=${html`<div class="r-empty">No context for this view.</div>`}>
        <${Match} when=${() => selectedNode().type === "artifact"}>
          <${RightArtifact}/>
        <//>
        <${Match} when=${() => selectedNode().type === "receipt"}>
          <${RightReceipt}/>
        <//>
        <${Match} when=${() => ["charter", "frame", "reviewer", "action"].includes(selectedNode().type)}>
          <${RightCharter}/>
        <//>
      <//>
    </div>
  `;
}

function RightArtifact() {
  const aid = () => selectedNode().id;
  const tied = createMemo(() => {
    const id = aid();
    return visibleReceipts().filter(
      (r) => r.tool_call?.params?.artifact_id === id,
    );
  });
  const tiedFindings = createMemo(() =>
    visibleFindings().filter((f) => fieldString(f.artifact_id) === aid()),
  );
  return html`
    <div style="padding:6px 10px;">
      <div style="font-size:11px;text-transform:uppercase;letter-spacing:0.05em;color:var(--t2);font-weight:600;margin-bottom:4px;">Artifact</div>
      <div style="font-family:var(--mono);font-size:11px;color:var(--t3);word-break:break-all;">${aid}</div>
    </div>
    <div style="padding:0 10px 4px;font-size:11px;text-transform:uppercase;letter-spacing:0.05em;color:var(--t2);font-weight:600;">Recent Receipts</div>
    <${Show} when=${() => tied().length > 0} fallback=${html`<div class="r-empty">No receipts yet.</div>`}>
      <div style="padding:0 8px;">
        <${For} each=${tied}>
          ${(r) => html`
            <div class="ctx-item" onClick=${() => navigate({ type: "receipt", id: r.receipt_id })}>
              <div class="ctx-h">
                <span class="ctx-t">${r.tool_call?.tool}</span>
                <span class="badge" style=${`color:${OUTCOME[r.outcome]?.fg};background:${OUTCOME[r.outcome]?.bg};`}>${r.outcome}</span>
              </div>
              <div class="ctx-meta">${tsLabel(r.timestamp)} · ${String(r.receipt_id).slice(0, 10)}…</div>
            </div>
          `}
        <//>
      </div>
    <//>
    <div style="padding:12px 10px 4px;font-size:11px;text-transform:uppercase;letter-spacing:0.05em;color:var(--t2);font-weight:600;">Recent Findings</div>
    <${Show} when=${() => tiedFindings().length > 0} fallback=${html`<div class="r-empty">No findings yet.</div>`}>
      <div style="padding:0 8px;">
        <${For} each=${tiedFindings}>
          ${(f) => html`
            <div class="ctx-item"
                 style=${`border-left-color:${SEV[f.severity] ?? "var(--border-a)"};`}
                 onClick=${() => navigate({ type: "finding", id: f.id })}>
              <div class="ctx-h">
                <span class="ctx-t">${f.severity}</span>
                <span class="ctx-l">${String(f.id).slice(0, 10)}</span>
              </div>
              <div class="ctx-b">${f.concern}</div>
            </div>
          `}
        <//>
      </div>
    <//>
  `;
}

function RightReceipt() {
  const cached = createMemo(() => receiptCache[selectedNode().id]);
  return html`
    <${Show}
      when=${() => cached()?.receipt}
      fallback=${html`<div class="r-empty">${() => cached()?.error ?? "Loading…"}</div>`}
    >
      ${() => {
        const r = cached().receipt;
        return html`
          <div style="padding:6px 10px;font-size:11px;text-transform:uppercase;letter-spacing:0.05em;color:var(--t2);font-weight:600;">Verdicts</div>
          <div style="padding:0 8px;">
            <${For} each=${r.verdicts ?? []}>
              ${(v) => html`
                <div class="ctx-item"
                     style=${`border-left-color:${RULING[v.ruling]?.fg ?? "var(--border-a)"};cursor:pointer;`}
                     onClick=${() => navigate({ type: "frame", id: `${fieldString(v.frame_ref?.steward_id)}/${fieldString(v.frame_ref?.frame_id)}` })}>
                  <div class="ctx-h">
                    <span class="ctx-t">${frameRefId(v)}</span>
                    <span class="badge" style=${`color:${RULING[v.ruling]?.fg};background:${RULING[v.ruling]?.bg};`}>${v.ruling}</span>
                  </div>
                  <div class="ctx-b">${v.reason || "—"}</div>
                </div>
              `}
            <//>
          </div>
        `;
      }}
    <//>
  `;
}

function RightCharter() {
  return html`
    <div style="padding:6px 10px;">
      <div style="font-size:11px;text-transform:uppercase;letter-spacing:0.05em;color:var(--t2);font-weight:600;margin-bottom:4px;">Charter</div>
      <div style="font-family:var(--mono);font-size:11px;color:var(--t3);word-break:break-all;">
        ${() => charter()?.charter_dir ?? ""}
      </div>
    </div>
    <${For} each=${() => charter()?.scopes ?? []}>
      ${(s) => html`
        <div class="scope-block charter" style="margin:6px 8px;">
          <div class="nm">${s.name}</div>
          <div class="bd" style="font-size:11px;line-height:1.5;">
            ${() => (s.text || "").slice(0, 240)}${() => ((s.text?.length ?? 0) > 240 ? "…" : "")}
          </div>
        </div>
      `}
    <//>
  `;
}

// ─────────────────────────────────────────────
// Banner stack
// ─────────────────────────────────────────────

function BannerStack() {
  return html`
    <div class="banner-stack">
      <${For} each=${banners}>
        ${(b) => html`
          <div class=${`banner ${b.kind === "warn" ? "warn" : b.kind === "danger" ? "danger" : ""}`}>
            <button class="close" onClick=${() => closeBanner(b.id)}>×</button>
            <div>${b.text}</div>
            <${Show} when=${b.receipt_id}>
              <div style="margin-top:4px;">
                <a href="#"
                   style="font-size:11px;color:inherit;text-decoration:underline;"
                   onClick=${(e) => { e.preventDefault(); navigate({ type: "receipt", id: b.receipt_id }); closeBanner(b.id); }}>
                  open Receipt ${String(b.receipt_id).slice(0, 10)}…
                </a>
              </div>
            <//>
          </div>
        `}
      <//>
    </div>
  `;
}

// ─────────────────────────────────────────────
// Resizable panel divider
// ─────────────────────────────────────────────

function Resizer(props) {
  const onPointerDown = (e) => {
    e.preventDefault();
    const startX = e.clientX;
    const target = props.side === "L" ? ".tree-pnl" : ".right-pnl";
    const el = document.querySelector(target);
    if (!el) return;
    const startWidth = el.getBoundingClientRect().width;
    const dir = props.side === "L" ? 1 : -1;
    const move = (ev) => {
      const next = Math.max(160, Math.min(640, startWidth + dir * (ev.clientX - startX)));
      el.style.width = `${next}px`;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  return html`<div class="dv" onPointerDown=${onPointerDown}></div>`;
}

// ─────────────────────────────────────────────
// Mount
// ─────────────────────────────────────────────

render(() => html`<${App}/>`, document.getElementById("app"));
