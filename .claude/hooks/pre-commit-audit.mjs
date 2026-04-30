#!/usr/bin/env node
// Pre-commit governance: mirrors CharteredOS runtime authority evaluation.
// Action (commit) → charter evaluation (claude -p) → verdict (allow/deny).
//
// AGENTS.md and CLAUDE.md are fed directly to the evaluator. The
// evaluator is instructed to read the documents they reference —
// docs/SPECIFICATION.md, docs/DESIGN_NOTES.md, docs/IMPLEMENTATION_CHECKLIST.md —
// itself for the governance model and implementation-review diagnostics.
//
// Uses git diff HEAD — PreToolUse fires before Bash executes, so a
// one-liner like `git add . && git commit ...` produces an empty diff
// at hook-fire time. The evaluator handles the empty case by reading
// the diff itself.
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const allow = () => JSON.stringify({
  hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "allow" }
});

const deny = (reason) => JSON.stringify({
  hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: reason }
});

try {
  // Read hook input from stdin to check the actual Bash command.
  // The "if" filter can't parse complex commands (&&, $(), pipes) so it
  // fires on all of them. We filter here instead.
  const input = readFileSync(0, "utf8");
  let hookInput;
  try { hookInput = JSON.parse(input); } catch { hookInput = {}; }
  const bashCmd = hookInput?.tool_input?.command ?? "";
  if (!bashCmd.includes("git commit") && !bashCmd.includes("git merge")) {
    // Not a commit — allow without auditing.
    console.log(allow());
    process.exit(0);
  }

  const root = execSync("git rev-parse --show-toplevel", { encoding: "utf8" }).trim();

  // Run CI sanity checks (cargo check + clippy + test) before governance audit.
  // Same script as .github/workflows/ci.yml — single source of truth.
  try {
    execSync(`${root}/scripts/ci-checks.sh`, { encoding: "utf8", stdio: "pipe", timeout: 300_000 });
  } catch (ciErr) {
    const output = (ciErr.stdout || "") + (ciErr.stderr || "");
    // Extract the relevant error lines (clippy/test failures)
    const lines = output.split("\n").filter(l => l.includes("error") || l.includes("FAILED")).slice(0, 10).join("\n");
    console.log(deny("CI checks failed:\n" + (lines || output.slice(-500))));
    process.exit(0);
  }

  // PreToolUse fires before the Bash command runs. A one-liner like
  // `git add . && git commit -m "..."` produces an empty `git diff HEAD`
  // at hook-fire time because the `add` has not happened yet. Do not
  // escape on empty — instruct the evaluator to read the diff itself.
  // Initial commit has no HEAD; `git diff HEAD` errors. Treat as empty
  // diff and let the evaluator read `git diff --cached` itself.
  let diff = "";
  try {
    diff = execSync("git diff HEAD", { encoding: "utf8" });
  } catch {
    diff = "";
  }

  const agentsMd = readFileSync(join(root, "AGENTS.md"), "utf8");
  const claudeMd = readFileSync(join(root, "CLAUDE.md"), "utf8");

  const diffSection = diff.trim()
    ? `---UNCOMMITTED DIFF---\n${diff}`
    : `---UNCOMMITTED DIFF---
(Empty at hook-fire time. The bash command may stage and commit in one step (\`git add . && git commit ...\`) so the diff is not yet visible to this hook. Run \`git diff HEAD\` and \`git diff --cached\` yourself in the repo root to read what is about to be committed, and evaluate that.)`;

  const prompt = `You are the CharteredOS pre-commit governance evaluator.

The bash command and the diff are below. Read CLAUDE.md and AGENTS.md (inlined) for the directive, then read the documents they reference — docs/SPECIFICATION.md, docs/DESIGN_NOTES.md, docs/IMPLEMENTATION_CHECKLIST.md — for the governance model and implementation-review diagnostics. RTFM before evaluating.

Two bash-command shapes that hide the staged diff from this audit and must be denied — the audit fires once per Bash invocation and inspects the diff at that moment, so staging within the same invocation hides what is being committed:

1. Combined staging and committing in one invocation. Recognize this semantically: the same bash command both stages files and creates the commit (chained with \`&&\`, \`;\`, or any equivalent), or otherwise causes staging and committing to happen in a single PreToolUse fire. The mention of "git add" inside a quoted commit message body is NOT this pattern.
   Reason to return: "Combined staging and committing in one Bash invocation hides the about-to-be-committed diff from the pre-commit audit. Split into two Bash tool calls: stage first, then commit."

2. Auto-staging during commit (\`git commit -a\`, \`git commit --all\`, or any flag that causes commit to stage modified files). The flag must be present in the actual command, not in a quoted message.
   Reason to return: "Auto-staging during commit hides the diff from the pre-commit audit. Stage with a separate Bash tool call (\`git add <paths>\`), then commit without auto-stage flags."

Otherwise: evaluate the bash command (the commit message it carries) and every + line in the diff against the directive and the diagnostics. Only flag clear, unambiguous breaches. Do not flag existing code or style preferences.

Reply with ONLY a JSON object:
- Clean: {"ok": true}
- Violation: {"ok": false, "reason": "§Section: what violates what, where, how to fix"}

---AGENTS.md---
${agentsMd}

---CLAUDE.md---
${claudeMd}

---BASH COMMAND---
${bashCmd}

${diffSection}`;

  const result = execSync("claude -p --output-format json", {
    input: prompt,
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 10 * 1024 * 1024,
  });

  let text;
  try {
    const sdk = JSON.parse(result);
    text = sdk.result || result;
  } catch {
    text = result;
  }

  const cleaned = text.replace(/^```(?:json)?\n?/gm, "").replace(/\n?```$/gm, "").trim();

  let parsed;
  try {
    parsed = JSON.parse(cleaned);
  } catch {
    console.log(deny("Governance evaluator returned non-JSON: " + cleaned.slice(0, 200)));
    process.exit(0);
  }

  if (parsed.ok === false) {
    console.log(deny(parsed.reason || "Engineering law violation detected"));
  } else {
    console.log(allow());
  }
} catch (err) {
  console.log(deny(err.message || "Pre-commit governance audit failed"));
  process.exit(0);
}
