export interface Piece {
  text: string;
  match: boolean;
}

const START = "\u0001";
const END = "\u0002";

/** Splits a search snippet into plain and highlighted pieces (matches are wrapped in U+0001 and U+0002). */
export function pieces(snippet: string): Piece[] {
  const out: Piece[] = [];
  let match = false;
  for (const part of snippet.split(new RegExp(`([${START}${END}])`))) {
    if (part === START) match = true;
    else if (part === END) match = false;
    else if (part) out.push({ text: part, match });
  }
  return out;
}
