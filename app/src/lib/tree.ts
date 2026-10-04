/** A folder or a file in the sidebar. */
export interface TreeNode {
  name: string;
  /** The full store path (for a folder, the path of the folder, without a trailing slash). */
  path: string;
  children?: TreeNode[];
}

/** Turns a flat list of store paths into folders and files: folders first, then by name, ignoring case. */
export function buildTree(paths: string[]): TreeNode[] {
  const root: TreeNode = { name: "", path: "", children: [] };
  for (const path of paths) {
    const parts = path.split("/");
    let node = root;
    parts.forEach((name, i) => {
      const full = parts.slice(0, i + 1).join("/");
      const last = i === parts.length - 1;
      node.children ??= [];
      let next = node.children.find((c) => c.name === name && !!c.children === !last);
      if (!next) {
        next = last ? { name, path: full } : { name, path: full, children: [] };
        node.children.push(next);
      }
      node = next;
    });
  }
  sort(root);
  return root.children ?? [];
}

function sort(node: TreeNode) {
  node.children?.sort((a, b) => {
    const folders = Number(!!b.children) - Number(!!a.children);
    return folders || a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
  });
  node.children?.forEach(sort);
}

export function isNote(path: string): boolean {
  return path.toLowerCase().endsWith(".md");
}

/** A note's title: its file name without the folder or the `.md`. */
export function title(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return isNote(name) ? name.slice(0, -3) : name;
}
