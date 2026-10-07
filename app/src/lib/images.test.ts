import { describe, expect, it } from "vitest";
import { imageMime, isImage, parseEmbed } from "./images";

describe("imageMime", () => {
  it("knows the usual picture types, whatever the case", () => {
    expect(imageMime("a.png")).toBe("image/png");
    expect(imageMime("Images/Pasted image 20250723195845.PNG")).toBe("image/png");
    expect(imageMime("x.jpg")).toBe("image/jpeg");
    expect(imageMime("x.JPEG")).toBe("image/jpeg");
    expect(imageMime("d.svg")).toBe("image/svg+xml");
    expect(imageMime("x.webp")).toBe("image/webp");
  });

  it("says no to everything else, including a dot in a folder name", () => {
    for (const path of ["note.md", "doc.pdf", "noextension", "folder.png/readme", "archive.png.zip", ""]) {
      expect(isImage(path), path).toBe(false);
    }
  });
});

describe("parseEmbed", () => {
  it("reads the file, a width and a description the way Obsidian writes them", () => {
    expect(parseEmbed("pic.png")).toEqual({ target: "pic.png", width: null, alt: "" });
    expect(parseEmbed("pic.png|300")).toEqual({ target: "pic.png", width: 300, alt: "" });
    expect(parseEmbed("pic.png|300x200")).toEqual({ target: "pic.png", width: 300, alt: "" });
    expect(parseEmbed("pic.png|a map of the forest")).toEqual({ target: "pic.png", width: null, alt: "a map of the forest" });
    expect(parseEmbed(" Pasted image 1.png ")).toMatchObject({ target: "Pasted image 1.png" });
    expect(parseEmbed("note#Heading").target).toBe("note");
  });
});
