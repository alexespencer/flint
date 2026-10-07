/** Which files the app can show as pictures, and how an embed like `![[pic.png|300]]` is read. */

const MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  bmp: "image/bmp",
  svg: "image/svg+xml",
};

/** The picture type for a path (by its extension), or `null` if it is not an image the app shows. */
export function imageMime(path: string): string | null {
  const dot = path.lastIndexOf(".");
  if (dot < 0 || path.lastIndexOf("/") > dot) return null;
  return MIME[path.slice(dot + 1).toLowerCase()] ?? null;
}

export function isImage(path: string): boolean {
  return imageMime(path) !== null;
}

export interface Embed {
  /** The file named, without a `#heading`. */
  target: string;
  /** A width in pixels, from `|300` or `|300x200` (the height is left to follow the width). */
  width: number | null;
  /** Any other text after the `|`: the picture's description. */
  alt: string;
}

/** Reads what is between the brackets of `![[ ... ]]`: `name.png`, `name.png|300`, `name.png|a description`. */
export function parseEmbed(inner: string): Embed {
  const [name, ...rest] = inner.split("|");
  const label = rest.join("|").trim();
  const size = /^(\d+)(?:x\d+)?$/.exec(label);
  return {
    target: name.split("#")[0].trim(),
    width: size ? Number(size[1]) : null,
    alt: size ? "" : label,
  };
}
