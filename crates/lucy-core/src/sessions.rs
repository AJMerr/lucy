//! Conversation history in a local SQLite database.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use lucy_protocol::rpc::ImageRef;
use lucy_protocol::{AgentLevel, Message, Mode, Role};
use rusqlite::{Connection, OptionalExtension, params};
use ulid::Ulid;

/// Bumped whenever the schema changes; each step in [`MIGRATIONS`] moves it up by one.
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE sessions (
        id          TEXT PRIMARY KEY,
        title       TEXT,
        created_at  INTEGER NOT NULL,  -- unix ms
        updated_at  INTEGER NOT NULL,  -- unix ms
        activity    INTEGER NOT NULL   -- monotonic counter, orders the list by recency
    );
    CREATE TABLE messages (
        session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        seq            INTEGER NOT NULL,
        role           TEXT NOT NULL,
        content        TEXT NOT NULL,
        profile        TEXT,
        model          TEXT,
        incomplete     INTEGER NOT NULL DEFAULT 0,
        input_tokens   INTEGER,
        output_tokens  INTEGER,
        created_at     INTEGER NOT NULL,
        PRIMARY KEY (session_id, seq)
    );
"#,
    r#"
    -- Tool use (step 8): what the assistant asked to run, and which call a result answers.
    ALTER TABLE messages ADD COLUMN tool_calls TEXT;  -- JSON array, assistant messages only
    ALTER TABLE messages ADD COLUMN tool_call_id TEXT;
    ALTER TABLE messages ADD COLUMN is_error INTEGER NOT NULL DEFAULT 0;
"#,
    r#"
    -- Per-chat settings (step 9); NULL means "use the config default".
    ALTER TABLE sessions ADD COLUMN profile TEXT;
    ALTER TABLE sessions ADD COLUMN plain INTEGER;
    ALTER TABLE sessions ADD COLUMN mode TEXT;
    ALTER TABLE sessions ADD COLUMN agent_level INTEGER;
    -- What files looked like before a reply changed them, for /undo.
    CREATE TABLE file_backups (
        session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        reply       TEXT NOT NULL,  -- ULID per reply, so they sort by time
        path        TEXT NOT NULL,
        content     BLOB,           -- NULL: the reply created the file
        PRIMARY KEY (session_id, reply, path)
    );
"#,
    r#"
    -- Pictures you attached: JSON array of {id, media_type}; the files are in <data>/images.
    ALTER TABLE messages ADD COLUMN images TEXT;
"#,
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(Ulid);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SessionId {
    type Err = StoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s)
            .map(Self)
            .map_err(|_| StoreError::NotFound(s.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id: SessionId,
    /// `None` until the first user message (or a rename).
    pub title: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub message_count: u32,
}

/// Which profile and model produced an assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub profile: String,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// A message to append to a session.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMessage {
    pub message: Message,
    pub origin: Option<Origin>,
    /// The reply was cut off (interrupted, connection lost, refused) before finishing.
    pub incomplete: bool,
    pub usage: Option<Usage>,
    /// Pictures you attached (kept in the core's image store).
    pub images: Vec<ImageRef>,
}

impl NewMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Message::new(Role::User, content), None)
    }

    /// Something you said, with pictures.
    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageRef>) -> Self {
        Self::user(content).with_images(images)
    }

    /// The same message with pictures that go with it.
    pub fn with_images(self, images: Vec<ImageRef>) -> Self {
        Self { images, ..self }
    }

    pub fn assistant(content: impl Into<String>, origin: Origin) -> Self {
        Self::new(Message::new(Role::Assistant, content), Some(origin))
    }

    /// A reply that asks to run tools (`content` is any text said first).
    pub fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<lucy_protocol::ToolCall>,
        origin: Origin,
    ) -> Self {
        Self::new(
            Message::assistant_with_tools(content, tool_calls),
            Some(origin),
        )
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::new(Message::tool_result(call_id, content, is_error), None)
    }

    fn new(message: Message, origin: Option<Origin>) -> Self {
        Self {
            message,
            origin,
            incomplete: false,
            usage: None,
            images: Vec::new(),
        }
    }

    pub fn incomplete(mut self) -> Self {
        self.incomplete = true;
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredMessage {
    pub message: Message,
    pub origin: Option<Origin>,
    pub incomplete: bool,
    pub usage: Option<Usage>,
    pub created_at: SystemTime,
    /// Pictures you attached (kept in the core's image store).
    pub images: Vec<ImageRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub summary: SessionSummary,
    pub messages: Vec<StoredMessage>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("no session matching `{0}`")]
    NotFound(String),
    #[error("`{prefix}` matches {count} sessions; type more of the ID")]
    Ambiguous { prefix: String, count: usize },
    #[error("could not create {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("session database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct SessionStore {
    conn: Connection,
}

/// `$LUCY_DATA_DIR/lucy.sqlite` if set, else `<platform data dir>/lucy/lucy.sqlite`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("LUCY_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|dir| dir.join("lucy")))
        .map(|dir| dir.join("lucy.sqlite"))
}

impl SessionStore {
    /// Open (creating if needed) the database at `path` and bring its schema up to date.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: parent.to_owned(),
                source,
            })?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn create(&self) -> Result<SessionId, StoreError> {
        let id = SessionId(Ulid::generate());
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, activity)
             VALUES (?1, ?2, ?2, (SELECT COALESCE(MAX(activity), 0) + 1 FROM sessions))",
            params![id.to_string(), now],
        )?;
        Ok(id)
    }

    /// All sessions, most recently active first.
    pub fn list(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.title, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.role != 'tool')
             FROM sessions s ORDER BY s.activity DESC",
        )?;
        let rows = stmt.query_map([], summary_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Add a message to the end of a session. The first user message also sets the title.
    pub fn append(&self, id: &SessionId, new: NewMessage) -> Result<(), StoreError> {
        let id_text = id.to_string();
        let now = now_ms();
        let touched = self.conn.execute(
            "UPDATE sessions
             SET updated_at = ?2,
                 activity = (SELECT MAX(activity) + 1 FROM sessions)
             WHERE id = ?1",
            params![id_text, now],
        )?;
        found(touched, id)?;

        let origin = new.origin.as_ref();
        let tool_calls = (!new.message.tool_calls.is_empty())
            .then(|| serde_json::to_string(&new.message.tool_calls).expect("tool calls serialize"));
        let images = (!new.images.is_empty())
            .then(|| serde_json::to_string(&new.images).expect("image refs serialize"));
        self.conn.execute(
            "INSERT INTO messages (session_id, seq, role, content, profile, model, incomplete,
                                   input_tokens, output_tokens, created_at, tool_calls,
                                   tool_call_id, is_error, images)
             VALUES (?1, (SELECT COUNT(*) FROM messages WHERE session_id = ?1),
                     ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id_text,
                role_name(new.message.role),
                new.message.content,
                origin.map(|o| &o.profile),
                origin.map(|o| &o.model),
                new.incomplete,
                new.usage.map(|u| u.input_tokens),
                new.usage.map(|u| u.output_tokens),
                now,
                tool_calls,
                new.message.tool_call_id,
                new.message.is_error,
                images,
            ],
        )?;

        if new.message.role == Role::User
            && let Some(title) = title_from(&new.message.content)
        {
            self.conn.execute(
                "UPDATE sessions SET title = ?2 WHERE id = ?1 AND title IS NULL",
                params![id_text, title],
            )?;
        }
        Ok(())
    }

    pub fn rename(&self, id: &SessionId, title: &str) -> Result<(), StoreError> {
        let changed = self.conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            params![id.to_string(), title],
        )?;
        found(changed, id)
    }

    /// This chat's own settings; `None` fields follow the config.
    pub fn settings(&self, id: &SessionId) -> Result<ChatSettings, StoreError> {
        self.conn
            .query_row(
                "SELECT profile, plain, mode, agent_level FROM sessions WHERE id = ?1",
                [id.to_string()],
                |row| {
                    let mode: Option<String> = row.get(2)?;
                    let level: Option<u8> = row.get(3)?;
                    Ok(ChatSettings {
                        profile: row.get(0)?,
                        plain: row.get(1)?,
                        mode: mode.map(|m| {
                            if m == "agent" {
                                Mode::Agent
                            } else {
                                Mode::Assist
                            }
                        }),
                        level: level.and_then(|l| AgentLevel::try_from(l).ok()),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    pub fn set_settings(&self, id: &SessionId, settings: &ChatSettings) -> Result<(), StoreError> {
        let changed = self.conn.execute(
            "UPDATE sessions SET profile = ?2, plain = ?3, mode = ?4, agent_level = ?5 WHERE id = ?1",
            params![
                id.to_string(),
                settings.profile,
                settings.plain,
                settings.mode.map(|m| match m {
                    Mode::Assist => "assist",
                    Mode::Agent => "agent",
                }),
                settings.level.map(u8::from),
            ],
        )?;
        found(changed, id)
    }

    /// Remember a file's contents (`None`: it didn't exist) before `reply` changes it. Only the
    /// first backup per file and reply counts, and only the last [`UNDO_REPLIES`] replies with
    /// changes are kept.
    pub fn record_backup(
        &self,
        id: &SessionId,
        reply: &str,
        path: &Path,
        before: Option<&[u8]>,
    ) -> Result<(), StoreError> {
        let id_text = id.to_string();
        self.conn.execute(
            "INSERT OR IGNORE INTO file_backups (session_id, reply, path, content)
             VALUES (?1, ?2, ?3, ?4)",
            params![id_text, reply, path.to_string_lossy(), before],
        )?;
        self.conn.execute(
            "DELETE FROM file_backups WHERE session_id = ?1 AND reply NOT IN (
                 SELECT DISTINCT reply FROM file_backups WHERE session_id = ?1
                 ORDER BY reply DESC LIMIT ?2)",
            params![id_text, UNDO_REPLIES],
        )?;
        Ok(())
    }

    /// The files the most recent recorded reply changed, and what they were before.
    pub fn last_backups(&self, id: &SessionId) -> Result<Vec<FileBackup>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT reply, path, content FROM file_backups
             WHERE session_id = ?1 AND reply = (
                 SELECT MAX(reply) FROM file_backups WHERE session_id = ?1)
             ORDER BY path",
        )?;
        let rows = stmt.query_map([id.to_string()], |row| {
            Ok(FileBackup {
                reply: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                before: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Forget one reply's backups (once they've been restored).
    pub fn drop_backups(&self, id: &SessionId, reply: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM file_backups WHERE session_id = ?1 AND reply = ?2",
            params![id.to_string(), reply],
        )?;
        Ok(())
    }

    /// Delete a session and all its messages.
    pub fn delete(&self, id: &SessionId) -> Result<(), StoreError> {
        let changed = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", [id.to_string()])?;
        found(changed, id)
    }

    /// The session whose ID starts with `prefix` (case-insensitive), if exactly one does.
    pub fn resolve(&self, prefix: &str) -> Result<SessionId, StoreError> {
        let prefix = prefix.trim().to_ascii_uppercase();
        let mut stmt = self.conn.prepare(
            "SELECT id FROM sessions WHERE substr(id, 1, length(?1)) = ?1 ORDER BY activity DESC",
        )?;
        let matches: Vec<String> = stmt
            .query_map([&prefix], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        match matches.as_slice() {
            [only] => only.parse(),
            [] => Err(StoreError::NotFound(prefix)),
            many => Err(StoreError::Ambiguous {
                prefix,
                count: many.len(),
            }),
        }
    }

    pub fn load(&self, id: &SessionId) -> Result<Session, StoreError> {
        let id_text = id.to_string();
        let summary = self
            .conn
            .query_row(
                "SELECT s.id, s.title, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.role != 'tool')
                 FROM sessions s WHERE s.id = ?1",
                [&id_text],
                summary_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(id_text.clone()))?;

        let mut stmt = self.conn.prepare(
            "SELECT role, content, profile, model, incomplete, input_tokens, output_tokens,
                    created_at, tool_calls, tool_call_id, is_error, images
             FROM messages WHERE session_id = ?1 ORDER BY seq",
        )?;
        let messages = stmt
            .query_map([&id_text], |row| {
                let role: String = row.get(0)?;
                let profile: Option<String> = row.get(2)?;
                let model: Option<String> = row.get(3)?;
                let input_tokens: Option<u32> = row.get(5)?;
                let output_tokens: Option<u32> = row.get(6)?;
                let tool_calls: Option<String> = row.get(8)?;
                let tool_calls = match tool_calls {
                    Some(json) => serde_json::from_str(&json).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            8,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?,
                    None => Vec::new(),
                };
                let images: Option<String> = row.get(11)?;
                let images = match images {
                    Some(json) => serde_json::from_str(&json).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            11,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?,
                    None => Vec::new(),
                };
                Ok(StoredMessage {
                    message: Message {
                        tool_calls,
                        tool_call_id: row.get(9)?,
                        is_error: row.get(10)?,
                        ..Message::new(parse_role(&role)?, row.get::<_, String>(1)?)
                    },
                    origin: profile
                        .zip(model)
                        .map(|(profile, model)| Origin { profile, model }),
                    incomplete: row.get(4)?,
                    usage: input_tokens.zip(output_tokens).map(|(i, o)| Usage {
                        input_tokens: i,
                        output_tokens: o,
                    }),
                    created_at: from_ms(row.get(7)?),
                    images,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(Session { summary, messages })
    }
}

fn found(rows_changed: usize, id: &SessionId) -> Result<(), StoreError> {
    if rows_changed == 0 {
        Err(StoreError::NotFound(id.to_string()))
    } else {
        Ok(())
    }
}

/// Longest title, ellipsis included.
const TITLE_MAX_CHARS: usize = 60;

/// First non-empty line of `text`, trimmed and shortened to [`TITLE_MAX_CHARS`].
fn title_from(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().count() <= TITLE_MAX_CHARS {
        return Some(line.to_owned());
    }
    let mut title: String = line.chars().take(TITLE_MAX_CHARS - 1).collect();
    title.push('…');
    Some(title)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn parse_role(name: &str) -> rusqlite::Result<Role> {
    Ok(match name {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown role `{other}`").into(),
            ));
        }
    })
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    let id: String = row.get(0)?;
    Ok(SessionSummary {
        id: SessionId(Ulid::from_string(&id).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
        })?),
        title: row.get(1)?,
        created_at: from_ms(row.get(2)?),
        updated_at: from_ms(row.get(3)?),
        message_count: row.get(4)?,
    })
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let tx = conn.transaction()?;
    for (i, migration) in (0..).zip(MIGRATIONS).skip(version as usize) {
        tx.execute_batch(migration)?;
        tx.pragma_update(None, "user_version", i + 1_i64)?;
    }
    tx.commit()?;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn from_ms(ms: i64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(ms.max(0) as u64)
}

/// How many replies' file changes `/undo` can step back through, per chat.
pub const UNDO_REPLIES: u32 = 20;

/// A chat's own choices; `None` means "follow the config".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatSettings {
    pub profile: Option<String>,
    pub plain: Option<bool>,
    pub mode: Option<Mode>,
    pub level: Option<AgentLevel>,
}

/// A file as it was before a reply changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBackup {
    pub reply: String,
    pub path: PathBuf,
    /// `None`: the reply created the file.
    pub before: Option<Vec<u8>>,
}

/// A new ID for a reply's backups (ULIDs sort by time).
pub fn new_reply_id() -> String {
    Ulid::generate().to_string()
}
