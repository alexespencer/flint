<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Annotation, EditorState, Prec, RangeSetBuilder } from "@codemirror/state";
  import {
    Decoration,
    EditorView,
    ViewPlugin,
    WidgetType,
    keymap,
    type DecorationSet,
    type ViewUpdate,
  } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { HighlightStyle, indentUnit, syntaxHighlighting, syntaxTree } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { isImage, parseEmbed } from "./lib/images";
  import { wikilinkAt } from "./lib/wikilink";

  let {
    text,
    readonly = false,
    onchange,
    onopenlink,
    loadImage,
    onpasteimages,
  }: {
    text: string;
    readonly?: boolean;
    onchange?: (text: string) => void;
    onopenlink?: (target: string) => void;
    /** Where to get the picture an embed names: a URL the page can show, or `null` if there is no such image. */
    loadImage?: (target: string) => Promise<string | null>;
    /** Pictures were pasted (or dropped, `dropped`) into the note: save them and return the text to insert (`![[name.png]]`). */
    onpasteimages?: (files: File[], dropped: boolean) => Promise<string>;
  } = $props();

  /** The pictures among some files from the clipboard or a drop. */
  const pictures = (files: FileList | undefined | null) => [...(files ?? [])].filter((file) => file.type.startsWith("image/"));

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

  /** A picture shown in place of `![[name.png]]` or `![alt](name.png)`. Clicking it opens it full size. */
  class ImageWidget extends WidgetType {
    target: string;
    width: number | null;
    alt: string;
    constructor(target: string, width: number | null, alt: string) {
      super();
      this.target = target;
      this.width = width;
      this.alt = alt;
    }
    eq(other: ImageWidget) {
      return other.target === this.target && other.width === this.width && other.alt === this.alt;
    }
    toDOM() {
      const holder = document.createElement("span");
      holder.className = "cm-embed-image cm-embed-loading";
      const image = document.createElement("img");
      image.alt = this.alt || this.target;
      if (this.width) image.style.width = `${this.width}px`;
      holder.append(image);
      holder.addEventListener("mousedown", (event) => event.preventDefault());
      holder.addEventListener("click", () => onopenlink?.(this.target));
      const failed = () => {
        holder.className = "cm-embed-image cm-embed-missing";
        holder.textContent = `Image not found: ${this.target}`;
      };
      if (!loadImage) failed();
      else {
        loadImage(this.target).then(
          (url) => {
            if (!url) return failed();
            image.onload = () => holder.classList.remove("cm-embed-loading");
            image.onerror = failed;
            image.src = url;
          },
          failed,
        );
      }
      return holder;
    }
    ignoreEvent() {
      return true;
    }
  }

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
        if (m[0].startsWith("!")) {
          const embed = parseEmbed(m[1]);
          if (isImage(embed.target)) {
            builder.add(start, end, Decoration.replace({ widget: new ImageWidget(embed.target, embed.width, embed.alt) }));
            continue;
          }
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

  /** `![alt](picture.png)`: the markdown parser finds the image and its address; only pictures inside the store are shown. */
  function markdownImageDecorations(view: EditorView): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const selection = view.state.selection.ranges;
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from,
        to,
        enter(node) {
          if (node.name !== "Image") return;
          const url = node.node.getChild("URL");
          if (!url || selection.some((r) => r.to >= node.from && r.from <= node.to)) return;
          const address = view.state.sliceDoc(url.from, url.to).trim();
          if (/^[a-z][a-z0-9+.-]*:/i.test(address)) return; // http:, data: ... are not loaded (nothing leaves the device)
          let target = address;
          try {
            target = decodeURIComponent(address);
          } catch {
            // not valid percent-encoding: use it as written
          }
          if (!isImage(target)) return;
          const marks = node.node.getChildren("LinkMark"); // `![`, `]`, `(`, `)`
          const alt = marks.length >= 2 ? view.state.sliceDoc(marks[0].to, marks[1].from) : "";
          builder.add(node.from, node.to, Decoration.replace({ widget: new ImageWidget(target, null, alt) }));
        },
      });
    }
    return builder.finish();
  }

  const imagePlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = markdownImageDecorations(view);
      }
      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet || update.viewportChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = markdownImageDecorations(update.view);
        }
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );

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

  // ---- task lists: `- [ ] item` shows a checkbox that can be clicked ------------------------------------

  /** Ticks or unticks the box whose `[` is at `pos`. */
  function toggleBox(view: EditorView, pos: number) {
    if (view.state.readOnly) return;
    const inner = view.state.sliceDoc(pos + 1, pos + 2);
    view.dispatch({ changes: { from: pos + 1, to: pos + 2, insert: inner === " " ? "x" : " " } });
  }

  class CheckboxWidget extends WidgetType {
    checked: boolean;
    constructor(checked: boolean) {
      super();
      this.checked = checked;
    }
    eq(other: CheckboxWidget) {
      return other.checked === this.checked;
    }
    toDOM(view: EditorView) {
      const box = document.createElement("input");
      box.type = "checkbox";
      box.className = "cm-task-checkbox";
      box.checked = this.checked;
      box.setAttribute("aria-label", this.checked ? "Done" : "Not done");
      box.tabIndex = -1; // Tab is for indenting the list, not for moving between boxes
      // keep the cursor where it is: a click should toggle, not move the caret
      box.addEventListener("mousedown", (event) => event.preventDefault());
      box.addEventListener("click", (event) => {
        event.preventDefault();
        toggleBox(view, view.posAtDOM(box));
      });
      return box;
    }
    ignoreEvent() {
      return true; // the checkbox handles its own events
    }
  }

  const doneText = Decoration.mark({ class: "cm-task-done" });

  /** A checkbox in place of each `- [ ]` / `- [x]`, unless the cursor is in it (then the source shows, to edit it). */
  function taskDecorations(view: EditorView): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const selection = view.state.selection.ranges;
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from,
        to,
        enter(node) {
          if (node.name !== "TaskMarker") return;
          const state = view.state;
          const checked = state.sliceDoc(node.from, node.to) !== "[ ]";
          const inMarker = selection.some((r) => r.from <= node.to && r.to >= node.from && !(r.empty && r.head === node.to));
          if (!inMarker) {
            // the bullet (`-`, `*`, `1.`) and the space after it give way to the box, as in Obsidian
            const bullet = node.node.parent?.parent?.firstChild;
            if (bullet?.name === "ListMark" && bullet.to < node.from) builder.add(bullet.from, node.from, Decoration.replace({}));
            builder.add(node.from, node.to, Decoration.replace({ widget: new CheckboxWidget(checked) }));
          }
          if (checked) {
            // dim and strike the task's text only, not the indent in front of it or the space after the box
            const end = state.doc.lineAt(node.from).to;
            let start = node.to;
            while (start < end && state.sliceDoc(start, start + 1) === " ") start++;
            if (start < end) builder.add(start, end, doneText);
          }
        },
      });
    }
    return builder.finish();
  }

  const taskPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = taskDecorations(view);
      }
      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet || update.viewportChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = taskDecorations(update.view);
        }
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );

  /** Ctrl/⌘+Enter ticks or unticks the task on the cursor's line. (Enter itself continues a checklist: the markdown
   *  mode's own keymap does that.) */
  function toggleTaskAtCursor(view: EditorView): boolean {
    if (view.state.readOnly) return false;
    const line = view.state.doc.lineAt(view.state.selection.main.head);
    let marker = -1;
    syntaxTree(view.state).iterate({
      from: line.from,
      to: line.to,
      enter(node) {
        if (node.name === "TaskMarker" && marker < 0) marker = node.from;
      },
    });
    if (marker < 0) return false;
    toggleBox(view, marker);
    return true;
  }

  /** Hides the `#`s (and the space after) of ATX headings, except on a line the cursor or selection touches. */
  function headingDecorations(view: EditorView): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const state = view.state;
    const selection = state.selection.ranges;
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(state).iterate({
        from,
        to,
        enter(node) {
          if (node.name !== "HeaderMark") return;
          const line = state.doc.lineAt(node.from);
          if (node.from !== line.from + (/^\s*/.exec(line.text)?.[0].length ?? 0)) return; // a closing `##`
          if (selection.some((r) => r.from <= line.to && r.to >= line.from)) return;
          let end = node.to;
          if (state.sliceDoc(end, end + 1) === " ") end++;
          if (end > node.from) builder.add(node.from, end, hidden);
        },
      });
    }
    return builder.finish();
  }

  const headingPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = headingDecorations(view);
      }
      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet || update.viewportChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = headingDecorations(update.view);
        }
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );

  const codeLine = Decoration.line({ class: "cm-codeblock" });
  const fenceHidden = Decoration.line({ class: "cm-codeblock cm-fence-hidden" });

  /**
   * Code, rendered: `inline` loses its backticks and a fenced block gets a background with its fence lines
   * collapsed, except where the cursor or selection touches it, so the source can be edited.
   */
  function codeDecorations(view: EditorView): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const state = view.state;
    const selection = state.selection.ranges;
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(state).iterate({
        from,
        to,
        enter(node) {
          if (node.name === "InlineCode") {
            if (selection.some((r) => r.from <= node.to && r.to >= node.from)) return false;
            for (let c = node.node.firstChild; c; c = c.nextSibling) {
              if (c.name === "CodeMark") builder.add(c.from, c.to, hidden);
            }
            return false;
          }
          if (node.name === "FencedCode") {
            const first = state.doc.lineAt(node.from).number;
            const last = state.doc.lineAt(node.to).number;
            const editing = selection.some((r) => r.from <= node.to && r.to >= node.from);
            const closed = node.node.lastChild?.name === "CodeMark" && last > first;
            for (let n = first; n <= last; n++) {
              const fence = n === first || (closed && n === last);
              builder.add(state.doc.line(n).from, state.doc.line(n).from, fence && !editing ? fenceHidden : codeLine);
            }
            return false;
          }
        },
      });
    }
    return builder.finish();
  }

  const codePlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = codeDecorations(view);
      }
      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet || update.viewportChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = codeDecorations(update.view);
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
    { tag: tags.monospace, fontFamily: "ui-monospace, Menlo, monospace", fontSize: "0.92em", backgroundColor: "var(--bg-side)", borderRadius: "3px" },
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
          markdown({ base: markdownLanguage }), // GitHub-flavoured: task lists, strikethrough, tables
          // Tab indents a list item (Shift+Tab outdents) instead of moving the browser's focus out of the editor
          indentUnit.of("    "),
          Prec.high(keymap.of([{ key: "Mod-Enter", run: toggleTaskAtCursor }, indentWithTab])),
          taskPlugin,
          headingPlugin,
          codePlugin,
          syntaxHighlighting(highlight),
          wikilinkPlugin,
          imagePlugin,
          EditorView.lineWrapping,
          EditorState.readOnly.of(readonly),
          EditorView.updateListener.of((u) => {
            if (u.docChanged && !u.transactions.some((t) => t.annotation(external))) {
              onchange?.(u.state.doc.toString());
            }
          }),
          EditorView.domEventHandlers({
            // a picture on the clipboard (a screenshot, "copy image") is saved into the store and linked
            paste(event, v) {
              const files = pictures(event.clipboardData?.files);
              if (files.length === 0 || !onpasteimages || v.state.readOnly) return false;
              event.preventDefault();
              onpasteimages(files, false).then((insert) => {
                if (insert) v.dispatch(v.state.replaceSelection(insert));
              });
              return true;
            },
            // picture files dragged in from the desktop are saved and linked where they are dropped
            drop(event, v) {
              const files = pictures(event.dataTransfer?.files);
              if (files.length === 0 || !onpasteimages || v.state.readOnly) return false;
              event.preventDefault();
              const at = v.posAtCoords({ x: event.clientX, y: event.clientY }) ?? v.state.selection.main.head;
              onpasteimages(files, true).then((insert) => {
                if (insert) v.dispatch({ changes: { from: at, insert }, selection: { anchor: at + insert.length } });
              });
              return true;
            },
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
  .editor :global(.cm-embed-image) {
    display: inline-block;
    max-width: 100%;
    cursor: zoom-in;
    vertical-align: top;
  }
  .editor :global(.cm-embed-image img) {
    display: block;
    max-width: 100%;
    max-height: 70vh;
    height: auto;
    border-radius: 6px;
  }
  .editor :global(.cm-embed-loading) {
    min-width: 120px;
    min-height: 60px;
    background: var(--bg-side);
    border-radius: 6px;
  }
  .editor :global(.cm-embed-missing) {
    cursor: default;
    color: var(--danger);
    font-size: 13px;
  }
  .editor :global(.cm-task-checkbox) {
    width: 1.05em;
    height: 1.05em;
    margin: 0 0.45em 0 0;
    vertical-align: -0.12em;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .editor :global(.cm-codeblock) {
    background: var(--bg-side);
    font-family: ui-monospace, Menlo, monospace;
    font-size: 0.92em;
  }
  .editor :global(.cm-fence-hidden) {
    display: none;
  }
  .editor :global(.cm-task-done) {
    color: var(--fg-dim);
    text-decoration: line-through;
  }
  /* a bigger target to tap */
  @media (hover: none) {
    .editor :global(.cm-task-checkbox) {
      width: 1.3em;
      height: 1.3em;
    }
  }
  .editor :global(.cm-wikilink-rendered) {
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--accent) 45%, transparent);
    text-underline-offset: 2px;
  }
</style>
