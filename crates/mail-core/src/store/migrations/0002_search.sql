DROP TABLE IF EXISTS message_fts;
CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, from_text, to_text, body_text,
  tokenize = 'unicode61 remove_diacritics 2'
);
