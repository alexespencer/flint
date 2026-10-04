/** A folder or a file in the sidebar. */
export interface TreeNode {
  name: string;
  /** The full store path (for a folder, the path of the folder, without a trailing slash). */
  path: string;
  children?: TreeNode[];
}

/**
 * Turns a flat list of store paths into folders and files: folders first, then by name, ignoring case.
 * `folders` are folders that exist on their own, so empty ones show up too.
 */
export function buildTree(paths: string[], folders: string[] = []): TreeNode[] {
  const root: TreeNode = { name: "", path: "", children: [] };
  for (const folder of folders) {
    let node = root;
    folder.split("/").forEach((name, i, parts) => {
      node.children ??= [];
      let next = node.children.find((c) => c.name === name && !!c.children);
      if (!next) {
        next = { name, path: parts.slice(0, i + 1).join("/"), children: [] };
        node.children.push(next);
      }
      node = next;
    });
  }
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

/** Whether `path` is `target` itself or something inside it (when `target` is a folder). */
export function isWithin(path: string, target: string): boolean {
  return path === target || path.startsWith(`${target}/`);
}

/** The folder a path is in: `a/b/c.md` is in `a/b`; a top-level file is in `` (the top level). */
export function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

/**
 * Where `path` (a file, or a folder) can be moved to: every folder except the one it is already in, itself
 * and (for a folder) anything inside itself, most recently created first. `""` is the top level, offered
 * last, unless it is where the thing already is.
 */
export function moveTargets(path: string, folders: { path: string; createdMs: number }[]): string[] {
  const here = parentOf(path);
  const targets = folders
    .filter((f) => f.path !== here && !isWithin(f.path, path))
    .sort((a, b) => b.createdMs - a.createdMs || a.path.localeCompare(b.path))
    .map((f) => f.path);
  return here === "" ? targets : [...targets, ""];
}

/** The part of a path after the last `/`. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** A copy that sync made of the other device's version of a conflicted note (`Note.conflict-<device>-<time>.md`). */
export function isConflictCopy(path: string): boolean {
  return /\.conflict-[^/]*$/.test(path);
}

export function isNote(path: string): boolean {
  return path.toLowerCase().endsWith(".md");
}

/** A note's title: its file name without the folder or the `.md`. */
export function title(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return isNote(name) ? name.slice(0, -3) : name;
}
