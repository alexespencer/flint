/// Bump when the tables below change. An index with another version is thrown away and rebuilt:
/// it is derived data, so there are no migrations.
pub(crate) const SCHEMA_VERSION: i64 = 1;

pub(crate) const SCHEMA: &str = "
CREATE TABLE files (
    path     TEXT PRIMARY KEY,
    stamp    TEXT NOT NULL,      -- opaque change marker from the caller (e.g. size+mtime or a hash)
    is_note  INTEGER NOT NULL,
    path_key TEXT NOT NULL,
    name_key TEXT NOT NULL,
    depth    INTEGER NOT NULL
);
CREATE INDEX files_name_key ON files(name_key);

CREATE TABLE notes (
    id                INTEGER PRIMARY KEY,
    path              TEXT NOT NULL UNIQUE REFERENCES files(path) ON DELETE CASCADE,
    frontmatter       TEXT,      -- JSON, so SQLite's json_extract can search properties later
    frontmatter_error TEXT
);

CREATE TABLE headings (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    ord     INTEGER NOT NULL,
    level   INTEGER NOT NULL,
    text    TEXT NOT NULL,
    PRIMARY KEY (note_id, ord)
);

CREATE TABLE tags (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag     TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    PRIMARY KEY (note_id, tag_key)
);
CREATE INDEX tags_tag_key ON tags(tag_key);

CREATE TABLE aliases (
    note_id   INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    ord       INTEGER NOT NULL,
    alias     TEXT NOT NULL,
    alias_key TEXT NOT NULL,
    PRIMARY KEY (note_id, ord)
);
CREATE INDEX aliases_alias_key ON aliases(alias_key);

CREATE TABLE links (
    note_id       INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    ord           INTEGER NOT NULL,
    kind          INTEGER NOT NULL,   -- 0 wikilink, 1 markdown
    embed         INTEGER NOT NULL,
    target        TEXT NOT NULL,
    fragment_kind INTEGER,            -- NULL none, 0 heading, 1 block
    fragment      TEXT,
    display       TEXT,
    range_start   INTEGER NOT NULL,
    range_end     INTEGER NOT NULL,
    leaf_key      TEXT NOT NULL,      -- what backlinks() looks up; resolution then confirms
    PRIMARY KEY (note_id, ord)
);
CREATE INDEX links_leaf_key ON links(leaf_key);

-- rowid = notes.id
CREATE VIRTUAL TABLE note_fts USING fts5(name, headings, tags, body, tokenize = 'unicode61 remove_diacritics 2');
";
