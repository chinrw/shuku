CREATE TABLE archive (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    quick_hash TEXT NOT NULL,
    content_hash TEXT,
    page_count INTEGER NOT NULL,
    date_added INTEGER NOT NULL,
    source_url TEXT,
    is_new INTEGER NOT NULL,
    lrr_id TEXT
);

CREATE TABLE tag (
    id INTEGER PRIMARY KEY,
    namespace TEXT NOT NULL,
    value TEXT NOT NULL,
    full TEXT NOT NULL UNIQUE
);

CREATE TABLE archive_tag (
    tag_id INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    PRIMARY KEY (tag_id, archive_id)
) WITHOUT ROWID;
CREATE INDEX archive_tag_by_archive ON archive_tag (archive_id, tag_id);

CREATE TABLE page (
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    page_no INTEGER NOT NULL,
    entry_name TEXT NOT NULL,
    data_offset INTEGER NOT NULL,
    compressed_size INTEGER NOT NULL,
    size INTEGER NOT NULL,
    method TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    PRIMARY KEY (archive_id, page_no)
);

CREATE TABLE progress (
    archive_id INTEGER PRIMARY KEY REFERENCES archive(id) ON DELETE CASCADE,
    page INTEGER NOT NULL,
    client_time INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE category (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    search TEXT NOT NULL,
    pinned INTEGER NOT NULL
);

CREATE TABLE category_archive (
    category_id INTEGER NOT NULL REFERENCES category(id) ON DELETE CASCADE,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    PRIMARY KEY (category_id, archive_id)
);

CREATE TABLE tankoubon (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    summary TEXT NOT NULL
);

CREATE TABLE tankoubon_archive (
    tankoubon_id INTEGER NOT NULL REFERENCES tankoubon(id) ON DELETE CASCADE,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (tankoubon_id, archive_id)
);

CREATE TABLE tankoubon_tag (
    tankoubon_id INTEGER NOT NULL REFERENCES tankoubon(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (tankoubon_id, tag_id)
);
