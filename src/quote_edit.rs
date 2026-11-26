//! Quote numbers, editing, undo and edit history.
//!
//! Quotes live in MySQL (`masterlist_quotes`). Every edit and undo is logged in
//! the bot's SQLite database (`quote_edits`) so it can be reversed and listed.
//! The two databases can't share a transaction, so a log row is written first
//! and removed again if the MySQL update doesn't happen. An edit that can't be
//! logged is refused, because it could never be undone.
//!
//! MySQL updates are conditional on the quote still holding the values we read
//! (compared byte for byte, since the column collation ignores case and
//! accents). If someone changed it in the meantime, nothing is overwritten.

use crate::database::DatabaseManager;
use crate::masterlist::{self, ShowLookup};
use anyhow::Result;
use chrono::DateTime;
use mysql::{prelude::*, Pool, PooledConn};
use serenity::all::{CreateAllowedMentions, CreateMessage, Http};
use serenity::model::channel::Message;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tracing::{error, info};

/// How many changes `!quote -history` lists.
pub const MAX_HISTORY: usize = 5;
/// Longest quote excerpt shown in an edit or undo confirmation.
const CONFIRM_SNIPPET: usize = 800;
/// Longest quote excerpt shown per history line.
const HISTORY_SNIPPET: usize = 120;
/// Discord's message limit.
const DISCORD_MAX: usize = 2000;

pub const EDIT_USAGE: &str =
    "Usage: !quote -edit <number> \"new text\" [-show <show>] [-episode <ep>]";
const FIND_NUMBER_HINT: &str = "Find a quote's number with !quote -id <search>.";
const ACTIONS: [&str; 3] = ["-edit", "-undo", "-history"];

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `!quote -id 1234`
    Show {
        id: u64,
    },
    Edit {
        id: u64,
        text: Option<String>,
        show: Option<String>,
        episode: Option<String>,
    },
    Undo {
        id: u64,
    },
    History {
        id: u64,
    },
    Invalid(String),
}

/// `1234` or `#1234` (the `#` is how numbers are displayed).
pub fn parse_quote_number(s: &str) -> Option<u64> {
    let digits = s.strip_prefix('#').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&n| n > 0)
}

/// Does a `!quote` command use `-edit`, `-undo` or `-history`?
pub fn mentions_action(args: &[&str]) -> bool {
    args.iter().any(|a| ACTIONS.contains(a))
}

/// Parse the tokens of a `!quote` command that uses `-edit`, `-undo` or `-history`.
pub fn parse_edit_command(tokens: &[String]) -> Command {
    let actions: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| ACTIONS.contains(&t.as_str()))
        .map(|(i, _)| i)
        .collect();
    if actions.len() != 1 {
        return Command::Invalid("Use only one of -edit, -undo or -history at a time.".into());
    }
    if let Some(flag) = tokens
        .iter()
        .find(|t| matches!(t.as_str(), "-add" | "-dud" | "-id"))
    {
        return Command::Invalid(format!(
            "{flag} can't be combined with -edit, -undo or -history."
        ));
    }

    let at = actions[0];
    let action = tokens[at].as_str();
    let Some(id) = tokens.get(at + 1).and_then(|t| parse_quote_number(t)) else {
        let example = if action == "-edit" {
            " \"new text\""
        } else {
            ""
        };
        return Command::Invalid(format!(
            "{action} needs a quote number, e.g. !quote {action} 1234{example}. {FIND_NUMBER_HINT}"
        ));
    };
    let rest: Vec<&String> = tokens[..at].iter().chain(&tokens[at + 2..]).collect();

    match action {
        "-undo" | "-history" if !rest.is_empty() => Command::Invalid(format!(
            "{action} takes only a quote number, e.g. !quote {action} {id}"
        )),
        "-undo" => Command::Undo { id },
        "-history" => Command::History { id },
        _ => parse_edit_rest(id, &rest),
    }
}

fn parse_edit_rest(id: u64, rest: &[&String]) -> Command {
    let mut show = None;
    let mut episode = None;
    let mut text_parts: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let token = rest[i].as_str();
        let slot = match token {
            "-show" => Some(&mut show),
            "-episode" | "-ep" => Some(&mut episode),
            _ => None,
        };
        match slot {
            Some(slot) => {
                let Some(value) = rest.get(i + 1).filter(|v| !v.starts_with('-')) else {
                    return Command::Invalid(format!("{token} needs a value. {EDIT_USAGE}"));
                };
                if slot.is_some() {
                    return Command::Invalid(format!("{token} was given twice. {EDIT_USAGE}"));
                }
                *slot = Some(value.to_string());
                i += 2;
            }
            None => {
                text_parts.push(token);
                i += 1;
            }
        }
    }
    let text = text_parts.join(" ").trim().to_string();
    let text = (!text.is_empty()).then_some(text);
    if text.is_none() && show.is_none() && episode.is_none() {
        return Command::Invalid(format!(
            "Nothing to change: give new text, -show or -episode. {EDIT_USAGE}"
        ));
    }
    Command::Edit {
        id,
        text,
        show,
        episode,
    }
}

// ---------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------

/// The parts of a quote an edit can change, plus the show title for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub text: String,
    pub show_id: i64,
    pub show_title: String,
    pub show_ep: String,
}

impl Version {
    /// Same text and attribution, byte for byte.
    fn same_content(&self, other: &Version) -> bool {
        self.text == other.text && self.show_id == other.show_id && self.show_ep == other.show_ep
    }

    fn attribution(&self) -> String {
        format!("{} {}", self.show_title, self.show_ep)
    }
}

#[derive(Debug, Clone)]
pub struct QuoteRow {
    pub id: u64,
    pub version: Version,
    pub episode_title: Option<String>,
}

/// Read one quote. Shows and episodes are left-joined, so a quote whose
/// episode was never listed is still found.
pub fn fetch_quote(conn: &mut PooledConn, id: u64) -> mysql::Result<Option<QuoteRow>> {
    type Row = (u64, String, i64, Option<String>, String, Option<String>);
    let row: Option<Row> = conn.exec_first(
        "SELECT q.quote_id, q.quote, q.show_id, s.show_title, q.show_ep, \
                (SELECT e.title FROM masterlist_episodes e \
                  WHERE e.show_id = q.show_id AND e.show_ep = q.show_ep \
                  ORDER BY e.episode_id LIMIT 1) \
           FROM masterlist_quotes q \
           LEFT JOIN masterlist_shows s ON s.show_id = q.show_id \
          WHERE q.quote_id = ?",
        (id,),
    )?;
    Ok(row.map(
        |(id, text, show_id, show_title, show_ep, episode_title)| QuoteRow {
            id,
            version: Version {
                text,
                show_id,
                show_title: show_title.unwrap_or_else(|| format!("show {show_id}")),
                show_ep,
            },
            episode_title: episode_title.filter(|t| !t.is_empty()),
        },
    ))
}

/// What an edit would do, worked out before anything is written.
#[derive(Debug)]
pub enum Prep {
    NotFound,
    /// The `-show` value didn't name exactly one show; holds the message.
    BadShow(String),
    EpisodeNotFound {
        show_title: String,
        episode: String,
    },
    NoChange,
    Ready {
        current: Version,
        new: Version,
    },
}

pub fn prepare_edit(
    conn: &mut PooledConn,
    id: u64,
    text: Option<&str>,
    show: Option<&str>,
    episode: Option<&str>,
) -> mysql::Result<Prep> {
    let Some(current) = fetch_quote(conn, id)? else {
        return Ok(Prep::NotFound);
    };
    let current = current.version;

    let (show_id, show_title) = match show {
        Some(query) => match masterlist::resolve_show(conn, query)? {
            ShowLookup::Found { id, title } => (id, title),
            lookup => {
                let message = masterlist::show_problem_message(query, &lookup)
                    .unwrap_or_else(|| format!("No show found matching '{query}'."));
                return Ok(Prep::BadShow(message));
            }
        },
        None => (current.show_id, current.show_title.clone()),
    };

    // A new attribution must name a listed episode. A text-only edit keeps the
    // existing one unchecked, so old quotes on unlisted episodes stay editable.
    let show_ep = if show.is_some() || episode.is_some() {
        let wanted = episode.unwrap_or(&current.show_ep);
        let listed: Option<String> = conn.exec_first(
            "SELECT show_ep FROM masterlist_episodes WHERE show_id = ? AND show_ep = ? \
             ORDER BY episode_id LIMIT 1",
            (show_id, wanted),
        )?;
        match listed {
            Some(code) => code,
            None => {
                return Ok(Prep::EpisodeNotFound {
                    show_title,
                    episode: wanted.to_string(),
                })
            }
        }
    } else {
        current.show_ep.clone()
    };

    let new = Version {
        text: text.map_or_else(|| current.text.clone(), str::to_string),
        show_id,
        show_title,
        show_ep,
    };
    if new.same_content(&current) {
        return Ok(Prep::NoChange);
    }
    Ok(Prep::Ready { current, new })
}

/// Change a quote from `expected` to `new`, only if it still holds `expected`.
/// Returns whether it was changed. `time = time` keeps the original timestamp.
pub fn apply_version(
    conn: &mut PooledConn,
    id: u64,
    expected: &Version,
    new: &Version,
) -> mysql::Result<bool> {
    conn.exec_drop(
        "UPDATE masterlist_quotes SET quote = ?, show_id = ?, show_ep = ?, time = time \
          WHERE quote_id = ? \
            AND CAST(quote AS BINARY) = CAST(? AS BINARY) \
            AND show_id = ? \
            AND CAST(show_ep AS BINARY) = CAST(? AS BINARY)",
        (
            &new.text,
            new.show_id,
            &new.show_ep,
            id,
            &expected.text,
            expected.show_id,
            &expected.show_ep,
        ),
    )?;
    Ok(conn.affected_rows() == 1)
}

fn quote_exists(conn: &mut PooledConn, id: u64) -> mysql::Result<bool> {
    let found: Option<u64> = conn.exec_first(
        "SELECT quote_id FROM masterlist_quotes WHERE quote_id = ?",
        (id,),
    )?;
    Ok(found.is_some())
}

/// Run blocking MySQL work on a pooled connection. Errors come back as the
/// message to show the user.
async fn with_mysql<T, F>(pool: &Pool, f: F) -> std::result::Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut PooledConn) -> mysql::Result<T> + Send + 'static,
{
    let pool = pool.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get_conn().map_err(|e| {
            error!("Failed to get MySQL connection for a quote edit: {:?}", e);
            "Failed to connect to the quote database.".to_string()
        })?;
        f(&mut conn).map_err(|e| {
            error!("Database error during a quote edit: {:?}", e);
            masterlist::describe_db_error(&e)
        })
    })
    .await;
    outcome.unwrap_or_else(|_| Err("The database task failed unexpectedly.".to_string()))
}

// ---------------------------------------------------------------------------
// SQLite edit log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Edit,
    Undo,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub id: i64,
    pub action: Action,
    pub editor_name: String,
    pub created_at: i64,
    pub old: Version,
    pub new: Version,
    /// For edits: whether a later undo reversed it.
    pub undone: bool,
}

/// Who ran the command.
#[derive(Debug, Clone)]
pub struct Editor {
    pub id: String,
    pub name: String,
}

pub async fn initialize_log_table(conn: &Connection) -> Result<()> {
    conn.call(|conn| {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS quote_edits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                quote_id INTEGER NOT NULL,
                action TEXT NOT NULL CHECK (action IN ('edit', 'undo')),
                editor_id TEXT NOT NULL,
                editor_name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                old_text TEXT NOT NULL,
                old_show_id INTEGER NOT NULL,
                old_show_title TEXT NOT NULL,
                old_show_ep TEXT NOT NULL,
                new_text TEXT NOT NULL,
                new_show_id INTEGER NOT NULL,
                new_show_title TEXT NOT NULL,
                new_show_ep TEXT NOT NULL,
                undone INTEGER NOT NULL DEFAULT 0,
                undoes_edit_id INTEGER REFERENCES quote_edits(id)
            );
            CREATE INDEX IF NOT EXISTS idx_quote_edits_quote ON quote_edits (quote_id, id);",
        )
    })
    .await?;
    Ok(())
}

const ENTRY_COLUMNS: &str = "id, action, editor_name, created_at, \
     old_text, old_show_id, old_show_title, old_show_ep, \
     new_text, new_show_id, new_show_title, new_show_ep, undone";

fn entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<LogEntry> {
    let action: String = row.get(1)?;
    Ok(LogEntry {
        id: row.get(0)?,
        action: if action == "undo" {
            Action::Undo
        } else {
            Action::Edit
        },
        editor_name: row.get(2)?,
        created_at: row.get(3)?,
        old: Version {
            text: row.get(4)?,
            show_id: row.get(5)?,
            show_title: row.get(6)?,
            show_ep: row.get(7)?,
        },
        new: Version {
            text: row.get(8)?,
            show_id: row.get(9)?,
            show_title: row.get(10)?,
            show_ep: row.get(11)?,
        },
        undone: row.get::<_, i64>(12)? != 0,
    })
}

fn insert_entry(
    tx: &rusqlite::Connection,
    quote_id: u64,
    action: &str,
    editor: &Editor,
    old: &Version,
    new: &Version,
    undoes_edit_id: Option<i64>,
) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO quote_edits (quote_id, action, editor_id, editor_name, created_at, \
            old_text, old_show_id, old_show_title, old_show_ep, \
            new_text, new_show_id, new_show_title, new_show_ep, undoes_edit_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            quote_id as i64,
            action,
            editor.id,
            editor.name,
            chrono::Utc::now().timestamp(),
            old.text,
            old.show_id,
            old.show_title,
            old.show_ep,
            new.text,
            new.show_id,
            new.show_title,
            new.show_ep,
            undoes_edit_id,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

pub async fn log_edit(
    conn: &Connection,
    quote_id: u64,
    editor: &Editor,
    old: &Version,
    new: &Version,
) -> Result<i64> {
    let (editor, old, new) = (editor.clone(), old.clone(), new.clone());
    Ok(conn
        .call(move |conn| insert_entry(conn, quote_id, "edit", &editor, &old, &new, None))
        .await?)
}

async fn delete_entry(conn: &Connection, id: i64) -> Result<()> {
    conn.call(move |conn| conn.execute("DELETE FROM quote_edits WHERE id = ?1", [id]))
        .await?;
    Ok(())
}

/// The most recent edit of a quote that hasn't been undone.
pub async fn latest_undoable(conn: &Connection, quote_id: u64) -> Result<Option<LogEntry>> {
    Ok(conn
        .call(move |conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENTRY_COLUMNS} FROM quote_edits \
                  WHERE quote_id = ?1 AND action = 'edit' AND undone = 0 \
                  ORDER BY id DESC LIMIT 1"
            ))?;
            let mut rows = stmt.query_map([quote_id as i64], entry_from_row)?;
            rows.next().transpose()
        })
        .await?)
}

/// Mark an edit undone and log the undo, in one SQLite transaction. Fails if
/// another undo got there first. Returns the undo's log id.
async fn begin_undo(
    conn: &Connection,
    quote_id: u64,
    edit: &LogEntry,
    editor: &Editor,
) -> Result<i64> {
    let edit_id = edit.id;
    let (edit, editor) = (edit.clone(), editor.clone());
    let undo_id = conn
        .call(move |conn| {
            let tx = conn.transaction()?;
            let marked = tx.execute(
                "UPDATE quote_edits SET undone = 1 WHERE id = ?1 AND undone = 0",
                [edit.id],
            )?;
            if marked != 1 {
                return Ok(None);
            }
            let id = insert_entry(
                &tx,
                quote_id,
                "undo",
                &editor,
                &edit.new,
                &edit.old,
                Some(edit.id),
            )?;
            tx.commit()?;
            Ok::<_, rusqlite::Error>(Some(id))
        })
        .await?;
    undo_id.ok_or_else(|| anyhow::anyhow!("edit log entry {edit_id} was already undone"))
}

/// Reverse [`begin_undo`] after the MySQL update didn't happen.
async fn cancel_undo(conn: &Connection, edit_id: i64, undo_id: i64) -> Result<()> {
    conn.call(move |conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM quote_edits WHERE id = ?1", [undo_id])?;
        tx.execute("UPDATE quote_edits SET undone = 0 WHERE id = ?1", [edit_id])?;
        tx.commit()
    })
    .await?;
    Ok(())
}

/// The most recent changes to a quote (newest first), and how many there are.
pub async fn history(
    conn: &Connection,
    quote_id: u64,
    limit: usize,
) -> Result<(Vec<LogEntry>, i64)> {
    Ok(conn
        .call(move |conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM quote_edits WHERE quote_id = ?1",
                [quote_id as i64],
                |r| r.get(0),
            )?;
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENTRY_COLUMNS} FROM quote_edits WHERE quote_id = ?1 ORDER BY id DESC LIMIT ?2"
            ))?;
            let entries = stmt
                .query_map(rusqlite::params![quote_id as i64, limit as i64], entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<_, rusqlite::Error>((entries, total))
        })
        .await?)
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Quote text as shown in Discord: entities from the old website decoded, cut
/// to `max` characters.
fn snippet(text: &str, max: usize) -> String {
    let text = html_escape::decode_html_entities(text);
    if text.chars().count() <= max {
        return text.into_owned();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

fn fmt_time(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "an unknown time".to_string())
}

fn fit_discord(mut text: String) -> String {
    if text.chars().count() > DISCORD_MAX {
        text = text.chars().take(DISCORD_MAX - 1).collect::<String>() + "…";
    }
    text
}

pub fn format_quote(row: &QuoteRow) -> String {
    let title = row
        .episode_title
        .as_deref()
        .map(|t| format!(": {t}"))
        .unwrap_or_default();
    fit_discord(format!(
        "(Quote #{}) {} -- {}{title}",
        row.id,
        snippet(&row.version.text, DISCORD_MAX),
        row.version.attribution()
    ))
}

pub fn edit_message(id: u64, old: &Version, new: &Version) -> String {
    let mut lines = vec![if old.attribution() == new.attribution() {
        format!("Edited quote #{id} ({}).", new.attribution())
    } else {
        format!(
            "Edited quote #{id}: moved from {} to {}.",
            old.attribution(),
            new.attribution()
        )
    }];
    if old.text != new.text {
        lines.push(format!("Before: {}", snippet(&old.text, CONFIRM_SNIPPET)));
        lines.push(format!("After: {}", snippet(&new.text, CONFIRM_SNIPPET)));
    }
    lines.push(format!("Undo with !quote -undo {id}"));
    fit_discord(lines.join("\n"))
}

pub fn undo_message(id: u64, edit: &LogEntry) -> String {
    fit_discord(format!(
        "Undid {}'s edit of quote #{id} from {}. It now reads:\n{} -- {}",
        edit.editor_name,
        fmt_time(edit.created_at),
        snippet(&edit.old.text, CONFIRM_SNIPPET),
        edit.old.attribution()
    ))
}

fn describe_change(old: &Version, new: &Version) -> String {
    let mut parts = Vec::new();
    if old.text != new.text {
        parts.push(format!(
            "\"{}\" → \"{}\"",
            snippet(&old.text, HISTORY_SNIPPET),
            snippet(&new.text, HISTORY_SNIPPET)
        ));
    }
    if old.attribution() != new.attribution() {
        parts.push(format!("{} → {}", old.attribution(), new.attribution()));
    }
    parts.join("; ")
}

pub fn history_message(id: u64, entries: &[LogEntry], total: i64) -> String {
    let mut lines = vec![format!("Changes to quote #{id}, newest first:")];
    for entry in entries {
        let what = match (entry.action, entry.undone) {
            (Action::Edit, false) => "edited",
            (Action::Edit, true) => "edited (later undone)",
            (Action::Undo, _) => "undid an edit",
        };
        lines.push(format!(
            "• {}, {} {what}: {}",
            fmt_time(entry.created_at),
            entry.editor_name,
            describe_change(&entry.old, &entry.new)
        ));
    }
    if total > entries.len() as i64 {
        lines.push(format!(
            "Showing the {} most recent of {total} changes.",
            entries.len()
        ));
    }
    fit_discord(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

const NO_LOG: &str =
    "the bot's local database isn't available, and edits must be logged so they can be undone";

/// Run a command and return the reply text. Keeps Discord out so it can be tested.
pub async fn run(
    pool: Option<Pool>,
    log: Option<Arc<Connection>>,
    editor: Option<Editor>,
    command: Command,
) -> String {
    let id = match &command {
        Command::Invalid(message) => return message.clone(),
        Command::Show { id }
        | Command::Edit { id, .. }
        | Command::Undo { id }
        | Command::History { id } => *id,
    };
    let Some(pool) = pool else {
        return "MySQL database is not configured.".to_string();
    };
    let reply = match command {
        Command::Invalid(_) => unreachable!(),
        Command::Show { id } => show(&pool, id).await,
        Command::History { id } => match log {
            Some(log) => show_history(&pool, &log, id).await,
            None => {
                "Edit history isn't available: the bot's local database isn't set up.".to_string()
            }
        },
        Command::Edit {
            id,
            text,
            show,
            episode,
        } => match (log, editor) {
            (Some(log), Some(editor)) => edit(&pool, &log, &editor, id, text, show, episode).await,
            _ => format!("I can't edit quotes right now: {NO_LOG}."),
        },
        Command::Undo { id } => match (log, editor) {
            (Some(log), Some(editor)) => undo(&pool, &log, &editor, id).await,
            _ => format!("I can't undo edits right now: {NO_LOG}."),
        },
    };
    info!("Quote #{} command reply: {}", id, reply);
    reply
}

fn no_such_quote(id: u64) -> String {
    format!("There's no quote #{id}. {FIND_NUMBER_HINT}")
}

async fn show(pool: &Pool, id: u64) -> String {
    match with_mysql(pool, move |conn| fetch_quote(conn, id)).await {
        Ok(Some(row)) => format_quote(&row),
        Ok(None) => no_such_quote(id),
        Err(message) => message,
    }
}

async fn edit(
    pool: &Pool,
    log: &Connection,
    editor: &Editor,
    id: u64,
    text: Option<String>,
    show: Option<String>,
    episode: Option<String>,
) -> String {
    let prep = with_mysql(pool, move |conn| {
        prepare_edit(
            conn,
            id,
            text.as_deref(),
            show.as_deref(),
            episode.as_deref(),
        )
    })
    .await;
    let (current, new) = match prep {
        Err(message) => return message,
        Ok(Prep::NotFound) => return no_such_quote(id),
        Ok(Prep::BadShow(message)) => return message,
        Ok(Prep::EpisodeNotFound {
            show_title,
            episode,
        }) => {
            let show_arg = masterlist::command_arg(&show_title);
            return format!(
                "No episode '{episode}' found for {show_title}. Give the episode with -episode <code>; \
                 see the codes with !episodes -show {show_arg}."
            );
        }
        Ok(Prep::NoChange) => {
            return format!("Quote #{id} already reads that way, so nothing changed.")
        }
        Ok(Prep::Ready { current, new }) => (current, new),
    };

    let log_id = match log_edit(log, id, editor, &current, &new).await {
        Ok(log_id) => log_id,
        Err(e) => {
            error!("Failed to log the edit of quote #{}: {:?}", id, e);
            return format!("I couldn't record the edit, so I didn't change quote #{id}.");
        }
    };

    let (expected, update) = (current.clone(), new.clone());
    let applied = with_mysql(pool, move |conn| {
        apply_version(conn, id, &expected, &update)
    })
    .await;
    if !matches!(applied, Ok(true)) {
        if let Err(e) = delete_entry(log, log_id).await {
            error!(
                "Failed to remove log entry {} for an edit that didn't happen: {:?}",
                log_id, e
            );
        }
    }
    match applied {
        Ok(true) => edit_message(id, &current, &new),
        Ok(false) => format!(
            "Quote #{id} changed while I was editing it, so I left it alone. Please try again."
        ),
        Err(message) => message,
    }
}

async fn undo(pool: &Pool, log: &Connection, editor: &Editor, id: u64) -> String {
    let edit = match latest_undoable(log, id).await {
        Ok(Some(edit)) => edit,
        Ok(None) => {
            return match with_mysql(pool, move |conn| quote_exists(conn, id)).await {
                Ok(true) => format!("Quote #{id} has no edits to undo."),
                Ok(false) => no_such_quote(id),
                Err(message) => message,
            }
        }
        Err(e) => {
            error!("Failed to read the edit log for quote #{}: {:?}", id, e);
            return format!("I couldn't read the edit history of quote #{id}.");
        }
    };

    let undo_id = match begin_undo(log, id, &edit, editor).await {
        Ok(undo_id) => undo_id,
        Err(e) => {
            error!("Failed to log the undo of quote #{}: {:?}", id, e);
            return format!("I couldn't record the undo, so I didn't change quote #{id}.");
        }
    };

    let (expected, restore) = (edit.new.clone(), edit.old.clone());
    let applied = with_mysql(pool, move |conn| {
        apply_version(conn, id, &expected, &restore)
    })
    .await;
    if !matches!(applied, Ok(true)) {
        if let Err(e) = cancel_undo(log, edit.id, undo_id).await {
            error!(
                "Failed to roll back the undo log for quote #{}: {:?}",
                id, e
            );
        }
    }
    match applied {
        Ok(true) => undo_message(id, &edit),
        Ok(false) => format!(
            "Quote #{id} has changed since {}'s edit of {}, so I left it alone. See !quote -history {id}.",
            edit.editor_name,
            fmt_time(edit.created_at)
        ),
        Err(message) => message,
    }
}

async fn show_history(pool: &Pool, log: &Connection, id: u64) -> String {
    match history(log, id, MAX_HISTORY).await {
        Ok((entries, total)) if !entries.is_empty() => history_message(id, &entries, total),
        Ok(_) => match with_mysql(pool, move |conn| quote_exists(conn, id)).await {
            Ok(true) => format!("Quote #{id} has no recorded edits."),
            Ok(false) => no_such_quote(id),
            Err(message) => message,
        },
        Err(e) => {
            error!("Failed to read the edit log for quote #{}: {:?}", id, e);
            format!("I couldn't read the edit history of quote #{id}.")
        }
    }
}

/// Run a command and reply in Discord. Quote text is user-supplied, so the
/// reply pings only the person who asked, never anyone named in a quote.
pub async fn handle(
    db: &DatabaseManager,
    log: Option<Arc<Connection>>,
    http: &Http,
    msg: &Message,
    editor: Option<Editor>,
    command: Command,
) -> Result<()> {
    info!("Quote command: {:?}", command);
    let text = run(db.pool.clone(), log, editor, command).await;
    let builder = CreateMessage::new()
        .content(text)
        .reference_message(msg)
        .allowed_mentions(CreateAllowedMentions::new().replied_user(true));
    msg.channel_id.send_message(http, builder).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        crate::Bot::tokenize_quote_args(s)
    }

    fn version(text: &str, show_id: i64, show: &str, ep: &str) -> Version {
        Version {
            text: text.into(),
            show_id,
            show_title: show.into(),
            show_ep: ep.into(),
        }
    }

    fn editor(name: &str) -> Editor {
        Editor {
            id: format!("id-{name}"),
            name: name.into(),
        }
    }

    #[test]
    fn quote_numbers() {
        assert_eq!(parse_quote_number("1234"), Some(1234));
        assert_eq!(parse_quote_number("#1234"), Some(1234));
        for bad in [
            "",
            "#",
            "0",
            "12a",
            "-5",
            "1.5",
            "clown",
            "99999999999999999999999",
        ] {
            assert_eq!(parse_quote_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_edit_undo_and_history() {
        assert_eq!(
            parse_edit_command(&toks("-edit 12 \"New  text\"")),
            Command::Edit {
                id: 12,
                text: Some("New  text".into()),
                show: None,
                episode: None
            }
        );
        // Unquoted words are joined; the number may be written as displayed.
        assert_eq!(
            parse_edit_command(&toks(
                "-edit #12 fixed the typo -show \"Space Ghost\" -ep 101"
            )),
            Command::Edit {
                id: 12,
                text: Some("fixed the typo".into()),
                show: Some("Space Ghost".into()),
                episode: Some("101".into())
            }
        );
        // Re-attribution alone, flags in any order.
        assert_eq!(
            parse_edit_command(&toks("-episode 0102 -edit 7")),
            Command::Edit {
                id: 7,
                text: None,
                show: None,
                episode: Some("0102".into())
            }
        );
        assert_eq!(
            parse_edit_command(&toks("-undo 5")),
            Command::Undo { id: 5 }
        );
        assert_eq!(
            parse_edit_command(&toks("-history #5")),
            Command::History { id: 5 }
        );
        // Text that merely starts with a dash is still text.
        assert!(matches!(
            parse_edit_command(&toks("-edit 3 \"-- Crow\"")),
            Command::Edit { text: Some(t), .. } if t == "-- Crow"
        ));
    }

    #[test]
    fn rejects_malformed_commands_with_a_hint() {
        let invalid = |s: &str| match parse_edit_command(&toks(s)) {
            Command::Invalid(m) => m,
            other => panic!("{s:?} parsed as {other:?}"),
        };
        assert!(invalid("-edit \"text\"").contains("needs a quote number"));
        assert!(invalid("-edit clown text").contains("!quote -id <search>"));
        assert!(invalid("-edit 12").contains("Nothing to change"));
        assert!(invalid("-edit 12 -show").contains("-show needs a value"));
        assert!(invalid("-edit 12 -show -episode 3").contains("-show needs a value"));
        assert!(invalid("-edit 12 -ep 1 -episode 2").contains("given twice"));
        assert!(invalid("-undo 12 please").contains("takes only a quote number"));
        assert!(invalid("-undo").contains("-undo needs a quote number"));
        assert!(invalid("-edit 1 x -undo 1").contains("only one of"));
        assert!(invalid("-edit 1 x -add").contains("-add can't be combined"));
        assert!(invalid("-history 1 -id").contains("-id can't be combined"));
    }

    #[test]
    fn dispatch_recognizes_the_action_flags() {
        assert!(mentions_action(&["-edit", "1", "x"]));
        assert!(mentions_action(&["clown", "-history"]));
        assert!(!mentions_action(&["-id", "clown", "-show", "mst3k"]));
        assert!(!mentions_action(&["-add", "-show", "x"]));
    }

    #[test]
    fn messages_describe_the_change_and_fit_discord() {
        let old = version("teh quote", 1, "MST3K", "0101");
        let new = version("the quote", 1, "MST3K", "0101");
        let m = edit_message(42, &old, &new);
        assert!(m.starts_with("Edited quote #42 (MST3K 0101)."), "{m}");
        assert!(m.contains("Before: teh quote\nAfter: the quote"), "{m}");
        assert!(m.ends_with("Undo with !quote -undo 42"), "{m}");

        let moved = version("teh quote", 1, "MST3K", "0102");
        let m = edit_message(42, &old, &moved);
        assert!(m.contains("moved from MST3K 0101 to MST3K 0102"), "{m}");
        assert!(!m.contains("Before:"), "{m}");

        // Entities from the old website are shown decoded.
        assert!(
            edit_message(1, &old, &version("&quot;Hi&quot; &amp; bye", 1, "S", "1"))
                .contains("After: \"Hi\" & bye")
        );

        // Two maximum-length quotes still make one Discord message.
        let long = "x".repeat(2000);
        let m = edit_message(
            1,
            &version(&long, 1, "S", "1"),
            &version(&format!("{long}y"), 1, "S", "1"),
        );
        assert!(m.chars().count() <= DISCORD_MAX, "{}", m.chars().count());
        assert!(m.contains('…'));

        let row = QuoteRow {
            id: 9,
            version: version("Hello", 1, "SG", "101"),
            episode_title: None,
        };
        assert_eq!(format_quote(&row), "(Quote #9) Hello -- SG 101");
    }

    #[tokio::test]
    async fn edit_log_supports_undo_in_order_and_history() {
        let log = Connection::open_in_memory().await.unwrap();
        initialize_log_table(&log).await.unwrap();
        initialize_log_table(&log).await.unwrap(); // idempotent

        let (a, b, c) = (
            version("A", 1, "S", "1"),
            version("B", 1, "S", "1"),
            version("C", 1, "S", "2"),
        );
        let e1 = log_edit(&log, 7, &editor("ann"), &a, &b).await.unwrap();
        let e2 = log_edit(&log, 7, &editor("bo"), &b, &c).await.unwrap();
        log_edit(&log, 8, &editor("cy"), &a, &c).await.unwrap(); // another quote

        // The newest edit is undone first, then the one before it.
        let latest = latest_undoable(&log, 7).await.unwrap().unwrap();
        assert_eq!((latest.id, latest.new.text.as_str()), (e2, "C"));
        let u1 = begin_undo(&log, 7, &latest, &editor("dee")).await.unwrap();
        // Undoing the same edit twice is refused.
        assert!(begin_undo(&log, 7, &latest, &editor("dee")).await.is_err());
        assert_eq!(latest_undoable(&log, 7).await.unwrap().unwrap().id, e1);

        // A cancelled undo makes the edit undoable again.
        cancel_undo(&log, e2, u1).await.unwrap();
        assert_eq!(latest_undoable(&log, 7).await.unwrap().unwrap().id, e2);

        let latest = latest_undoable(&log, 7).await.unwrap().unwrap();
        begin_undo(&log, 7, &latest, &editor("dee")).await.unwrap();
        let (entries, total) = history(&log, 7, MAX_HISTORY).await.unwrap();
        assert_eq!(total, 3);
        let kinds: Vec<_> = entries
            .iter()
            .map(|e| (e.action, e.undone, e.editor_name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (Action::Undo, false, "dee"),
                (Action::Edit, true, "bo"),
                (Action::Edit, false, "ann")
            ]
        );
        // The undo restores what the edit replaced.
        assert_eq!(
            (entries[0].old.text.as_str(), entries[0].new.text.as_str()),
            ("C", "B")
        );

        let m = history_message(7, &entries, total);
        assert!(
            m.contains("dee undid an edit: \"C\" → \"B\"; S 2 → S 1"),
            "{m}"
        );
        assert!(m.contains("bo edited (later undone)"), "{m}");
        assert!(!m.contains("Showing the"), "{m}");
        let m = history_message(7, &entries[..2], total);
        assert!(
            m.ends_with("Showing the 2 most recent of 3 changes."),
            "{m}"
        );

        // Removing a log row for an edit that didn't happen.
        delete_entry(&log, e1).await.unwrap();
        assert_eq!(history(&log, 7, MAX_HISTORY).await.unwrap().1, 2);
    }

    #[tokio::test]
    async fn commands_explain_what_is_missing() {
        assert_eq!(
            run(None, None, None, Command::Show { id: 1 }).await,
            "MySQL database is not configured."
        );
        assert_eq!(
            run(None, None, None, Command::Invalid("nope".into())).await,
            "nope"
        );
    }

    // ---- MySQL (scratch database) ----------------------------------------

    use crate::masterlist::test_support::fresh_db;

    const KEEP_TIME: &str = "2001-02-03 04:05:06";

    /// Add a quote with a fixed old timestamp; returns its id.
    fn add_quote(pool: &Pool, show_id: i64, ep: &str, text: &str) -> u64 {
        let mut conn = pool.get_conn().unwrap();
        conn.exec_drop(
            "INSERT INTO masterlist_quotes (show_id, show_ep, quote, time) VALUES (?, ?, ?, ?)",
            (show_id, ep, text, KEEP_TIME),
        )
        .unwrap();
        conn.last_insert_id()
    }

    fn stored(pool: &Pool, id: u64) -> (String, i64, String, String) {
        let mut conn = pool.get_conn().unwrap();
        conn.exec_first(
            "SELECT quote, show_id, show_ep, CAST(time AS CHAR) FROM masterlist_quotes WHERE quote_id = ?",
            (id,),
        )
        .unwrap()
        .unwrap()
    }

    async fn sqlite_log() -> Option<Arc<Connection>> {
        let log = Connection::open_in_memory().await.unwrap();
        initialize_log_table(&log).await.unwrap();
        Some(Arc::new(log))
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    async fn db_edit_undo_and_history_round_trip() {
        let db = fresh_db();
        let pool = Some(db.pool.clone());
        let log = sqlite_log().await;
        let id = add_quote(&db.pool, 1, "0101", "I've lost my toupée");
        let ed = |name: &str| Some(editor(name));
        let edit = |text: Option<&str>, show: Option<&str>, ep: Option<&str>| Command::Edit {
            id,
            text: text.map(Into::into),
            show: show.map(Into::into),
            episode: ep.map(Into::into),
        };

        // Lookup by number, including the episode title.
        let shown = run(pool.clone(), None, None, Command::Show { id }).await;
        assert_eq!(
            shown,
            format!("(Quote #{id}) I've lost my toupée -- MST3K 0101: The Crawling Eye")
        );

        // A case-only change is a real edit (the column ignores case when comparing).
        let m = run(
            pool.clone(),
            log.clone(),
            ed("ann"),
            edit(Some("I've lost my Toupée"), None, None),
        )
        .await;
        assert!(
            m.starts_with(&format!("Edited quote #{id} (MST3K 0101).")),
            "{m}"
        );
        assert_eq!(stored(&db.pool, id).0, "I've lost my Toupée");
        // ...and the original timestamp is kept.
        assert_eq!(stored(&db.pool, id).3, KEEP_TIME);

        // Re-attribution validates the target, normalizing the episode code.
        let m = run(
            pool.clone(),
            log.clone(),
            ed("bo"),
            edit(None, Some("mst3k"), Some("s01")),
        )
        .await;
        assert!(m.contains("moved from MST3K 0101 to MST3K S01"), "{m}");
        assert_eq!(stored(&db.pool, id).2, "S01");

        // Nothing is written for a refused edit.
        let refusals = [
            (
                edit(Some("I've lost my Toupée"), None, None),
                "already reads that way",
            ),
            (
                edit(None, None, Some("9999")),
                "No episode '9999' found for MST3K",
            ),
            (edit(None, Some("mst3"), Some("0101")), "MST3K60m"),
            (
                edit(None, Some("nope"), Some("1")),
                "No show found matching 'nope'",
            ),
            // Moving to a show where the current episode code doesn't exist.
            (
                edit(None, Some("sg"), None),
                "Give the episode with -episode",
            ),
        ];
        for (command, expected) in refusals {
            let m = run(pool.clone(), log.clone(), ed("cy"), command).await;
            assert!(m.contains(expected), "{expected:?} not in {m:?}");
        }
        let missing = Command::Edit {
            id: 999_999,
            text: Some("x".into()),
            show: None,
            episode: None,
        };
        assert!(run(pool.clone(), log.clone(), ed("cy"), missing)
            .await
            .starts_with("There's no quote #999999"));
        assert_eq!(history(log.as_deref().unwrap(), id, 50).await.unwrap().1, 2);

        // Undo steps back one edit at a time, keeping the timestamp.
        let m = run(pool.clone(), log.clone(), ed("dee"), Command::Undo { id }).await;
        assert!(
            m.starts_with(&format!("Undid bo's edit of quote #{id}")),
            "{m}"
        );
        assert_eq!(
            stored(&db.pool, id),
            (
                "I've lost my Toupée".into(),
                1,
                "0101".into(),
                KEEP_TIME.into()
            )
        );
        let m = run(pool.clone(), log.clone(), ed("dee"), Command::Undo { id }).await;
        assert!(m.contains("ann's edit"), "{m}");
        assert_eq!(stored(&db.pool, id).0, "I've lost my toupée");
        let m = run(pool.clone(), log.clone(), ed("dee"), Command::Undo { id }).await;
        assert_eq!(m, format!("Quote #{id} has no edits to undo."));

        let m = run(pool.clone(), log.clone(), None, Command::History { id }).await;
        assert!(
            m.starts_with(&format!("Changes to quote #{id}, newest first:")),
            "{m}"
        );
        assert_eq!(m.matches("undid an edit").count(), 2, "{m}");
        // Four changes fit in one page, so there's no "showing N of M" line.
        assert!(!m.contains("Showing the"), "{m}");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    async fn db_undo_refuses_to_overwrite_a_later_change() {
        let db = fresh_db();
        let pool = Some(db.pool.clone());
        let log = sqlite_log().await;
        let id = add_quote(&db.pool, 1, "0101", "original");
        let edit = Command::Edit {
            id,
            text: Some("edited".into()),
            show: None,
            episode: None,
        };
        run(pool.clone(), log.clone(), Some(editor("ann")), edit).await;

        // Someone changes the quote outside the bot, differing only in case.
        let mut conn = db.pool.get_conn().unwrap();
        conn.exec_drop(
            "UPDATE masterlist_quotes SET quote = 'EDITED' WHERE quote_id = ?",
            (id,),
        )
        .unwrap();

        let m = run(
            pool.clone(),
            log.clone(),
            Some(editor("bo")),
            Command::Undo { id },
        )
        .await;
        assert!(m.contains("has changed since ann's edit"), "{m}");
        assert_eq!(stored(&db.pool, id).0, "EDITED");
        // The refused undo left no trace and the edit is still undoable.
        let (entries, total) = history(log.as_deref().unwrap(), id, 50).await.unwrap();
        assert_eq!((total, entries[0].undone), (1, false));

        // An edit based on a stale read is refused the same way.
        let current = fetch_quote(&mut conn, id).unwrap().unwrap().version;
        let stale = Version {
            text: "edited".into(),
            ..current.clone()
        };
        assert!(!apply_version(
            &mut conn,
            id,
            &stale,
            &Version {
                text: "new".into(),
                ..current
            }
        )
        .unwrap());
        assert_eq!(stored(&db.pool, id).0, "EDITED");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    async fn db_quotes_on_unlisted_episodes_stay_editable() {
        let db = fresh_db();
        let pool = Some(db.pool.clone());
        // Episode 7777 is not in masterlist_episodes (old data has such quotes).
        let id = add_quote(&db.pool, 1, "7777", "orphan");
        let shown = run(pool.clone(), None, None, Command::Show { id }).await;
        assert_eq!(shown, format!("(Quote #{id}) orphan -- MST3K 7777"));
        let edit = Command::Edit {
            id,
            text: Some("adopted".into()),
            show: None,
            episode: None,
        };
        let m = run(pool.clone(), sqlite_log().await, Some(editor("ann")), edit).await;
        assert!(
            m.starts_with(&format!("Edited quote #{id} (MST3K 7777).")),
            "{m}"
        );
    }
}
