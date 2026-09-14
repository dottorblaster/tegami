CREATE TABLE account (
  id            INTEGER PRIMARY KEY,
  source        TEXT NOT NULL,
  external_id   TEXT NOT NULL,
  email         TEXT NOT NULL,
  display_name  TEXT,
  imap_host     TEXT,
  imap_port     INTEGER,
  imap_security TEXT,
  smtp_host     TEXT,
  smtp_port     INTEGER,
  smtp_security TEXT,
  auth_kind     TEXT NOT NULL,
  username      TEXT,
  UNIQUE(source, external_id)
);

CREATE TABLE folder (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  name          TEXT NOT NULL,
  display_name  TEXT,
  special_use   TEXT,
  uidvalidity   INTEGER,
  uidnext       INTEGER,
  highestmodseq INTEGER,
  unread_count  INTEGER DEFAULT 0,
  total_count   INTEGER DEFAULT 0,
  subscribed    INTEGER DEFAULT 1,
  UNIQUE(account_id, name)
);

CREATE TABLE message (
  id            INTEGER PRIMARY KEY,
  folder_id     INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
  uid           INTEGER NOT NULL,
  modseq        INTEGER,
  message_id    TEXT,
  thread_id     INTEGER,
  subject       TEXT,
  from_addr     TEXT,
  from_name     TEXT,
  to_addrs      TEXT,
  cc_addrs      TEXT,
  date_sent     INTEGER,
  date_recv     INTEGER,
  in_reply_to   TEXT,
  refs          TEXT,
  flags         INTEGER DEFAULT 0,
  has_attach    INTEGER DEFAULT 0,
  size          INTEGER,
  structure     TEXT,
  raw_path      TEXT,
  body_state    TEXT DEFAULT 'none',
  UNIQUE(folder_id, uid)
);
CREATE INDEX idx_message_thread ON message(thread_id);
CREATE INDEX idx_message_msgid  ON message(message_id);
CREATE INDEX idx_message_date   ON message(folder_id, date_recv DESC);

CREATE TABLE attachment (
  id            INTEGER PRIMARY KEY,
  message_id    INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  part_id       TEXT,
  filename      TEXT,
  mime_type     TEXT,
  size          INTEGER,
  content_id    TEXT,
  disk_path     TEXT
);

CREATE TABLE outbox (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id),
  raw_path      TEXT NOT NULL,
  state         TEXT DEFAULT 'queued',
  send_after    INTEGER,
  attempts      INTEGER DEFAULT 0,
  last_error    TEXT,
  created_at    INTEGER
);

CREATE TABLE pending_op (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id),
  op_kind       TEXT NOT NULL,
  folder_id     INTEGER,
  target_folder_id INTEGER,
  uid           INTEGER,
  payload       TEXT,
  created_at    INTEGER
);

CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, from_text, to_text, body_text,
  content='',
  tokenize = 'unicode61 remove_diacritics 2'
);