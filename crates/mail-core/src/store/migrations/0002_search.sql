DROP TABLE IF EXISTS message_fts;
CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, from_text, to_text, body_text,
  tokenize = 'unicode61 remove_diacritics 2'
);
INSERT INTO message_fts (rowid, subject, from_text, to_text, body_text)
SELECT id,
       subject,
       COALESCE(from_name, '') || ' ' || COALESCE(from_addr, ''),
       COALESCE(to_addrs, '') || ' ' || COALESCE(cc_addrs, ''),
       ''
FROM message;