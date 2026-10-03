// M0 spike: is CodeMirror 6 usable in the Android WebView on a 5,000-line note?
// Measures keystroke-to-next-frame latency for every edit and shows median / p95 / worst.
import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";

const out = document.getElementById("out");
const stats = document.getElementById("stats");
const host = document.getElementById("editor");

const samples = [];
let pendingStart = null;

function report() {
  if (!samples.length) return;
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
  stats.textContent =
    `edits ${samples.length} · median ${at(0.5).toFixed(1)} ms · p95 ${at(0.95).toFixed(1)} ms · worst ${sorted.at(-1).toFixed(1)} ms`;
}

// Time from the browser's input event to the frame after CodeMirror has re-rendered.
const latency = EditorView.domEventHandlers({
  beforeinput() {
    pendingStart = performance.now();
  },
});
const measure = EditorView.updateListener.of((u) => {
  if (!u.docChanged || pendingStart === null) return;
  const start = pendingStart;
  pendingStart = null;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    samples.push(performance.now() - start);
    report();
  }));
});

function makeNote(lines) {
  const parts = [];
  let i = 0;
  while (parts.length < lines) {
    i++;
    parts.push(`## Section ${i}`, "");
    parts.push(`Some **bold** text, an _italic_ phrase, \`inline code\`, and a [[Wiki Link ${i}]] to another note.`);
    parts.push(`- [ ] task ${i} with a [markdown link](notes/item-${i}.md) and #tag${i % 17}`);
    parts.push("> a quoted line that wraps around when the screen is narrow enough to need it, as phones are");
    parts.push("");
  }
  return parts.slice(0, lines).join("\n");
}

let view = null;
function load(lines, highlight) {
  if (view) view.destroy();
  samples.length = 0;
  stats.textContent = "type in the editor to collect latency";
  const t0 = performance.now();
  view = new EditorView({
    parent: host,
    state: EditorState.create({
      doc: makeNote(lines),
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping,
        ...(highlight ? [markdown(), syntaxHighlighting(defaultHighlightStyle)] : []),
        latency,
        measure,
      ],
    }),
  });
  out.textContent = `${lines} lines${highlight ? " + markdown highlighting" : ", plain"} · created in ${(performance.now() - t0).toFixed(0)} ms`;
}

document.getElementById("l500").onclick = () => load(500, true);
document.getElementById("l5000").onclick = () => load(5000, true);
document.getElementById("l5000plain").onclick = () => load(5000, false);
document.getElementById("l20000").onclick = () => load(20000, true);
document.getElementById("probe").onclick = async () => {
  try {
    out.textContent = await window.__TAURI__.core.invoke("storage_probe");
  } catch (e) {
    out.textContent = "FAILED: " + e;
  }
};

// ---- Android Keystore spike -------------------------------------------------------------
const ks = (cmd, args) => window.__TAURI__.core.invoke(`plugin:keystore|${cmd}`, args);
const b64 = (u8) => btoa(String.fromCharCode(...u8));
const show = (label, v) => (out.textContent = `${label}: ${typeof v === "string" ? v : JSON.stringify(v, null, 1)}`);
const fail = (label) => (e) => show(`${label} FAILED`, String(e));

document.getElementById("ks-status").onclick = () => ks("status").then((r) => show("status", r), fail("status"));

// Wraps a fresh random 32-byte "DEK". The spike keeps the original in localStorage purely so
// unwrap can prove a byte-for-byte round trip; the real app never stores the plaintext key.
document.getElementById("ks-wrap").onclick = async () => {
  const dek = crypto.getRandomValues(new Uint8Array(32));
  try {
    const wrapped = await ks("wrap", { payload: { data: b64(dek) } });
    localStorage.setItem("ks-test", JSON.stringify({ wrapped, expected: b64(dek) }));
    show("wrap ok (touch sensor happened)", { ivBytes: atob(wrapped.iv).length, ciphertextBytes: atob(wrapped.ciphertext).length });
  } catch (e) {
    fail("wrap")(e);
  }
};
document.getElementById("ks-unwrap").onclick = async () => {
  const saved = JSON.parse(localStorage.getItem("ks-test") || "null");
  if (!saved) return show("unwrap", "nothing wrapped yet: press KS wrap first");
  try {
    const r = await ks("unwrap", { payload: saved.wrapped });
    show("unwrap", r.data === saved.expected ? "ok: key round-trips byte for byte" : "MISMATCH: wrong bytes came back");
  } catch (e) {
    fail("unwrap")(e);
  }
};
document.getElementById("ks-forget").onclick = () =>
  ks("forget").then(() => { localStorage.removeItem("ks-test"); show("forget", "key deleted"); }, fail("forget"));

load(500, true);
