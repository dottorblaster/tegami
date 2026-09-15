-- Senders whose remote content the reader opted into loading.
CREATE TABLE IF NOT EXISTS remote_content_sender (
  id     INTEGER PRIMARY KEY,
  sender TEXT NOT NULL UNIQUE
);
