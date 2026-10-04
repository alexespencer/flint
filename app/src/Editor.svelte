<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Annotation, EditorState, RangeSetBuilder } from "@codemirror/state";
  import {
    Decoration,
    EditorView,
    ViewPlugin,
    keymap,
    type DecorationSet,
    type ViewUpdate,
  } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
  import { markdown } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { wikilinkAt } from "./lib/wikilink";

  let {
    text,
    readonly = false,
    onchange,
    onopenlink,
  }: {
    text: string;
    readonly?: boolean;
    onchange?: (text: string) => void;
    onopenlink?: (target: string) => void;
  } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | undefined;

  /** Marks a change that came from outside (a sync brought newer text), so it is not reported as an edit. */
  const external = Annotation.define<boolean>();

  // When the text passed in changes under us (another device's edit arrived), show it, keeping the cursor.
  $effect(() => {
    const incoming = text;
    if (!view || incoming === view.state.doc.toString()) return;
    const head = Math.min(view.state.selection.main.head, incoming.length);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: incoming },
      selection: { anchor: head },
      annotations: external.of(true),
    });
  });

  const hidden = Decoration.replace({});
  const editingLink = Decoration.mark({ class: "cm-wikilink" });
  const renderedLink = Decoration.mark({ class: "cm-wikilink cm-wikilink-rendered" });

  /**
   * Shows `[[Note]]` as a plain link (brackets hidden, `[[Note|alias]]` as the alias) unless the cursor or
   * selection touches it, when the source is shown so it can be edited.
   */
  function linkDecorations(view: EditorView): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const selection = view.state.selection.ranges;
    for (const { from, to } of view.visibleRanges) {
      for (const m of view.state.doc.sliceString(from, to).matchAll(/!?\[\[([^\]\n]+?)\]\]/g)) {
        const start = from + m.index!;
        const end = start + m[0].length;
        if (selection.some((r) => r.to >= start && r.from <= end)) {
          builder.add(start, end, editingLink);
          continue;
        }
        const open = start + (m[0].startsWith("!") ? 3 : 2);
        const bar = m[1].indexOf("|");
        builder.add(start, bar >= 0 ? open + bar + 1 : open, hidden);
        builder.add(bar >= 0 ? open + bar + 1 : open, end - 2, renderedLink);
        builder.add(end - 2, end, hidden);
      }
    }
    return builder.finish();
  }

  const wikilinkPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = linkDecorations(view);
      }
      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet || update.viewportChanged) {
          this.decorations = linkDecorations(update.view);
        }
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );

  const highlight = HighlightStyle.define([
    { tag: tags.heading1, fontSize: "1.55em", fontWeight: "700" },
    { tag: tags.heading2, fontSize: "1.3em", fontWeight: "700" },
    { tag: tags.heading3, fontSize: "1.15em", fontWeight: "700" },
    { tag: [tags.heading4, tags.heading5, tags.heading6], fontWeight: "700" },
    { tag: tags.strong, fontWeight: "700" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: tags.link, color: "var(--accent)" },
    { tag: tags.url, color: "var(--fg-dim)" },
    { tag: tags.monospace, fontFamily: "ui-monospace, Menlo, monospace", fontSize: "0.92em" },
    { tag: tags.quote, color: "var(--fg-dim)", fontStyle: "italic" },
    { tag: tags.processingInstruction, color: "var(--fg-dim)" },
  ]);

  onMount(() => {
    view = new EditorView({
      parent: host!,
      state: EditorState.create({
        doc: text,
        extensions: [
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          markdown(),
          syntaxHighlighting(highlight),
          wikilinkPlugin,
          EditorView.lineWrapping,
          EditorState.readOnly.of(readonly),
          EditorView.updateListener.of((u) => {
            if (u.docChanged && !u.transactions.some((t) => t.annotation(external))) {
              onchange?.(u.state.doc.toString());
            }
          }),
          EditorView.domEventHandlers({
            // A click on a rendered link follows it; Cmd/Ctrl+click follows one whose source is showing
            mousedown(event, v) {
              const rendered = event.target instanceof Element && !!event.target.closest(".cm-wikilink-rendered");
              if (!rendered && !(event.metaKey || event.ctrlKey)) return false;
              const pos = v.posAtCoords({ x: event.clientX, y: event.clientY });
              if (pos == null) return false;
              const line = v.state.doc.lineAt(pos);
              const target = wikilinkAt(line.text, pos - line.from);
              if (!target) return false;
              event.preventDefault();
              onopenlink?.(target);
              return true;
            },
          }),
        ],
      }),
    });
    if (!readonly) view.focus();
  });

  onDestroy(() => view?.destroy());
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    overflow: auto;
  }
  .editor :global(.cm-editor) {
    min-height: 100%;
    background: transparent;
    color: var(--fg);
  }
  .editor :global(.cm-editor.cm-focused) {
    outline: none;
  }
  .editor :global(.cm-scroller) {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    font-size: 15.5px;
    line-height: 1.65;
    padding: 18px 0 40vh;
  }
  .editor :global(.cm-content) {
    max-width: 760px;
    margin: 0 auto;
    padding: 0 28px;
    caret-color: var(--fg);
  }
  .editor :global(.cm-cursor) {
    border-left-color: var(--fg);
  }
  .editor :global(.cm-selectionBackground),
  .editor :global(.cm-focused .cm-selectionBackground) {
    background: var(--bg-active) !important;
  }
  /* The markdown highlighter also colours the brackets inside [[...]] as link syntax; a wikilink is one colour. */
  .editor :global(.cm-wikilink),
  .editor :global(.cm-wikilink *) {
    color: var(--accent) !important;
  }
  .editor :global(.cm-wikilink-rendered) {
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--accent) 45%, transparent);
    text-underline-offset: 2px;
  }
</style>
