/** The target of the `[[wikilink]]` that covers offset `pos` in `line`, if there is one. */
export function wikilinkAt(line: string, pos: number): string | null {
  const pattern = /!?\[\[([^\]\n]+?)\]\]/g;
  for (let m = pattern.exec(line); m; m = pattern.exec(line)) {
    if (pos >= m.index && pos <= m.index + m[0].length) {
      // [[Note#Heading|alias]] -> Note
      return m[1].split("|")[0].split("#")[0].trim() || null;
    }
  }
  return null;
}
