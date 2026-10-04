<script lang="ts">
  import FileTree from "./FileTree.svelte";
  import { isConflictCopy, isNote, title, type TreeNode } from "./lib/tree";

  let {
    nodes,
    selected,
    expanded,
    conflicted,
    ontoggle,
    onopen,
    onmenu,
    depth = 0,
  }: {
    nodes: TreeNode[];
    selected: string | null;
    expanded: Set<string>;
    conflicted: Set<string>;
    ontoggle: (path: string) => void;
    onopen: (path: string) => void;
    /** The person asked for the actions on a file or folder (right-click, or its ⋯ button). */
    onmenu: (node: TreeNode, x: number, y: number) => void;
    depth?: number;
  } = $props();
</script>

<ul style:--depth={depth}>
  {#each nodes as node (node.children ? `d:${node.path}` : `f:${node.path}`)}
    <li>
      {#if node.children}
        <div class="line" oncontextmenu={(e) => (e.preventDefault(), onmenu(node, e.clientX, e.clientY))} role="presentation">
          <button class="row" onclick={() => ontoggle(node.path)} aria-expanded={expanded.has(node.path)}>
            <span class="chev">{expanded.has(node.path) ? "▾" : "▸"}</span>
            {node.name}
          </button>
          <button class="more" aria-label="Actions for {node.name}" onclick={(e) => onmenu(node, e.clientX, e.clientY)}>⋯</button>
        </div>
        {#if expanded.has(node.path)}
          <FileTree
            nodes={node.children}
            {selected}
            {expanded}
            {conflicted}
            {ontoggle}
            {onopen}
            {onmenu}
            depth={depth + 1}
          />
        {/if}
      {:else}
        <div class="line" oncontextmenu={(e) => (e.preventDefault(), onmenu(node, e.clientX, e.clientY))} role="presentation">
          <button
            class="row"
            class:selected={selected === node.path}
            class:attachment={!isNote(node.path)}
            class:copy={isConflictCopy(node.path)}
            disabled={!isNote(node.path)}
            onclick={() => onopen(node.path)}
            title={node.path}
          >
            <span class="chev"></span>
            {isConflictCopy(node.path) ? `${title(node.path).replace(/\.conflict-.*$/, "")} (conflict copy)` : title(node.path)}
            {#if conflicted.has(node.path)}<span class="badge" title="Conflict">!</span>{/if}
          </button>
          <button class="more" aria-label="Actions for {node.name}" onclick={(e) => onmenu(node, e.clientX, e.clientY)}>⋯</button>
        </div>
      {/if}
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .line {
    display: flex;
    align-items: center;
  }
  .more {
    flex: none;
    background: none;
    border: none;
    padding: 2px 8px;
    color: var(--fg-dim);
    opacity: 0;
  }
  .line:hover .more,
  .more:focus-visible {
    opacity: 1;
  }
  /* no hover on a touch screen, so the button is always there */
  @media (hover: none), (max-width: 720px) {
    .more {
      opacity: 1;
      padding: 6px 12px;
    }
  }
  .row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    background: none;
    border: none;
    border-radius: 5px;
    padding: 3px 8px 3px calc(8px + var(--depth) * 14px);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--bg-active);
    font-weight: 600;
  }
  .row.copy {
    color: var(--fg-dim);
    font-style: italic;
  }
  .row:disabled {
    color: var(--fg-dim);
  }
  .chev {
    display: inline-block;
    width: 14px;
    flex: none;
    color: var(--fg-dim);
    font-size: 11px;
  }
  .badge {
    margin-left: auto;
    background: var(--danger);
    color: var(--bg);
    border-radius: 50%;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 700;
  }
</style>
