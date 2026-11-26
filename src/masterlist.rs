//! Shows and episodes in the MySQL quote "masterlist".
//!
//! Provides the `!shows` and `!episodes` commands:
//!
//! * `!shows`                         list every show with its episode count
//! * `!shows -add <title>`            add a show
//! * `!episodes -show <show>`         list a show's episodes
//! * `!episodes -show <s> -episode <e>`  check whether an episode exists
//! * `!episodes -add -show <s> -episode <e> [title]`  add an episode
//!
//! A quote can only be added to an existing show and episode, so these commands
//! are the way to register a new one first.
//!
//! The module is split so most of it can be tested without Discord or MySQL:
//! parsing and formatting are pure functions, the SQL lives in small functions
//! that take a connection, and only [`handle`] touches serenity.

use crate::database::DatabaseManager;
use anyhow::Result;
use mysql::{prelude::*, Pool, PooledConn};
use serenity::all::Http;
use serenity::builder::{CreateAllowedMentions, CreateMessage};
use serenity::model::channel::Message;
use tracing::{error, info};

/// `masterlist_shows.show_title` is `varchar(255)`.
const MAX_SHOW_TITLE_CHARS: usize = 255;
/// `masterlist_episodes.title` is `varchar(255)`.
const MAX_EPISODE_TITLE_CHARS: usize = 255;
/// `masterlist_episodes.show_ep` is `varchar(10)`.
const MAX_EPISODE_CODE_CHARS: usize = 10;

/// Discord rejects messages over 2000 characters; stay safely under it.
const MESSAGE_BUDGET: usize = 1900;
/// A long listing is cut off after this many messages rather than flooding the channel.
const MAX_MESSAGES: usize = 5;
/// Room kept in each message for the "...and N more" note.
const TRUNCATION_NOTE_RESERVE: usize = 40;
/// How many candidate shows to name when a show reference is ambiguous.
const MAX_AMBIGUOUS_SHOWN: usize = 8;

const SHOWS_USAGE: &str = "Usage:\n\
!shows - list the shows\n\
!shows -add <title> - add a show, e.g. !shows -add \"Space Ghost\"\n\
More help: !help quotes";
const EPISODES_USAGE: &str = "Usage:\n\
!episodes -show <show> - list a show's episodes\n\
!episodes -show <show> -episode <ep> - check whether an episode exists\n\
!episodes -add -show <show> -episode <ep> [title] - add an episode, e.g. !episodes -add -show MST3K -episode 1001 \"Episode title\"\n\
More help: !help quotes";

/// The `!help quotes` walkthrough. Kept well under Discord's 2000-character limit.
pub const QUOTES_HELP: &str = "Adding a quote (it needs a show and an episode that already exist)\n\
1. Find the show: !shows (shows are short codes like MST3K; the number is how many episodes it has)\n\
2. Find the episode code: !episodes -show MST3K lists them. To check one: !episodes -show MST3K -episode 0101\n\
3. Show missing? Check !shows for another spelling first, then: !shows -add \"Show Title\"\n\
4. Episode missing? !episodes -add -show MST3K -episode 1001 \"Episode title\" (the title is optional)\n\
5. Add the quote: !quote -add -show MST3K -episode 0101 \"The quote text\"\n\
\n\
Fixing a quote\n\
1. Find its number: !quote -id <search> shows numbers. !quote -id 1234 shows one quote.\n\
2. Fix the text: !quote -edit 1234 \"The corrected text\"\n\
3. Or move it: !quote -edit 1234 -show MST3K -episode 0102\n\
4. Made a mistake? !quote -undo 1234 reverses the latest edit (run it again to go back further).\n\
5. !quote -history 1234 lists recent edits and who made them.\n\
\n\
Good to know\n\
- Put anything with spaces in \"double quotes\".\n\
- A show name can be shortened if it only matches one show. If it matches several (MST3K and MST3K60m), I'll list them so you can be more specific.\n\
- Episode codes are up to 10 characters with no spaces (0101, K00, S01, SP-3). Match the style of the show's existing codes.\n\
- There is no delete command, so double-check before adding.";

/// Is this the topic word in `!help <topic>` that should show [`QUOTES_HELP`]?
pub fn is_quotes_help_topic(topic: &str) -> bool {
    matches!(
        topic.to_lowercase().as_str(),
        "quotes"
            | "quote"
            | "shows"
            | "show"
            | "episodes"
            | "episode"
            | "edit"
            | "undo"
            | "history"
    )
}

/// Format a value for use inside a command we suggest to the user: names with
/// spaces need double quotes or they would be read as several arguments.
pub(crate) fn command_arg(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// A parsed `!shows` / `!episodes` command.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    ListShows,
    AddShow {
        title: String,
    },
    ListEpisodes {
        show: String,
    },
    CheckEpisode {
        show: String,
        episode: String,
    },
    AddEpisode {
        show: String,
        episode: String,
        title: String,
    },
    /// The command line was malformed; the text tells the user how to fix it.
    Invalid(String),
}

/// The part of a message after the command word: `"!shows -add Foo"` -> `"-add Foo"`.
pub fn args_text(content: &str) -> &str {
    let rest = content.trim_start();
    let rest = rest.strip_prefix('!').unwrap_or(rest);
    match rest.find(char::is_whitespace) {
        Some(i) => rest[i..].trim_start(),
        None => "",
    }
}

/// Parse the tokens of a `!shows` command.
pub fn parse_shows(tokens: &[String]) -> Command {
    match tokens.first().map(|t| t.to_lowercase()) {
        None => Command::ListShows,
        Some(flag) if flag == "-add" => {
            let title = tokens[1..].join(" ");
            match validate_show_title(&title) {
                Ok(title) => Command::AddShow { title },
                Err(e) => Command::Invalid(format!("{e}\n{SHOWS_USAGE}")),
            }
        }
        Some(_) => Command::Invalid(format!("!shows takes no search text.\n{SHOWS_USAGE}")),
    }
}

/// Parse the tokens of an `!episodes` command.
pub fn parse_episodes(tokens: &[String]) -> Command {
    let mut add = false;
    let mut show: Option<String> = None;
    let mut episode: Option<String> = None;
    let mut title_parts: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        let lower = tokens[i].to_lowercase();
        match lower.as_str() {
            "-add" => {
                add = true;
                i += 1;
            }
            "-show" | "-episode" | "-ep" => {
                let Some(value) = tokens.get(i + 1) else {
                    return Command::Invalid(format!("{lower} needs a value.\n{EPISODES_USAGE}"));
                };
                if lower == "-show" {
                    show = Some(value.clone());
                } else {
                    episode = Some(value.clone());
                }
                i += 2;
            }
            _ => {
                title_parts.push(&tokens[i]);
                i += 1;
            }
        }
    }

    let Some(show) = show else {
        // `!episodes mst3k` is the likely slip: say what was probably meant.
        let hint = if !add && episode.is_none() && !title_parts.is_empty() {
            format!(
                " Did you mean !episodes -show {}?",
                command_arg(&title_parts.join(" "))
            )
        } else {
            String::new()
        };
        return Command::Invalid(format!("-show <show> is required.{hint}\n{EPISODES_USAGE}"));
    };

    if add {
        let Some(episode) = episode else {
            return Command::Invalid(format!("-episode <ep> is required.\n{EPISODES_USAGE}"));
        };
        let episode = match validate_episode_code(&episode) {
            Ok(code) => code,
            Err(e) => return Command::Invalid(e),
        };
        let title = match validate_episode_title(&title_parts.join(" ")) {
            Ok(title) => title,
            Err(e) => return Command::Invalid(e),
        };
        return Command::AddEpisode {
            show,
            episode,
            title,
        };
    }

    if !title_parts.is_empty() {
        return Command::Invalid(format!(
            "Unexpected text '{}'. To add an episode, include -add.\n{EPISODES_USAGE}",
            title_parts.join(" ")
        ));
    }

    match episode {
        Some(episode) => Command::CheckEpisode { show, episode },
        None => Command::ListEpisodes { show },
    }
}

fn has_control_chars(s: &str) -> bool {
    s.chars().any(char::is_control)
}

fn validate_show_title(raw: &str) -> Result<String, String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err("A show title is required.".to_string());
    }
    if has_control_chars(title) {
        return Err("Show titles can't contain line breaks or control characters.".to_string());
    }
    if title.chars().count() > MAX_SHOW_TITLE_CHARS {
        return Err(format!(
            "Show titles are limited to {MAX_SHOW_TITLE_CHARS} characters."
        ));
    }
    Ok(title.to_string())
}

fn validate_episode_code(raw: &str) -> Result<String, String> {
    let code = raw.trim();
    if code.is_empty() {
        return Err("An episode code is required.".to_string());
    }
    if code.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Episode codes can't contain spaces or control characters.".to_string());
    }
    if code.chars().count() > MAX_EPISODE_CODE_CHARS {
        return Err(format!(
            "Episode codes are limited to {MAX_EPISODE_CODE_CHARS} characters."
        ));
    }
    Ok(code.to_string())
}

/// An episode title is optional: many existing episodes have none.
fn validate_episode_title(raw: &str) -> Result<String, String> {
    let title = raw.trim();
    if has_control_chars(title) {
        return Err("Episode titles can't contain line breaks or control characters.".to_string());
    }
    if title.chars().count() > MAX_EPISODE_TITLE_CHARS {
        return Err(format!(
            "Episode titles are limited to {MAX_EPISODE_TITLE_CHARS} characters."
        ));
    }
    Ok(title.to_string())
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Pack `items` (joined by `sep`) into messages of at most `budget` characters.
/// The first message starts with `header`. If more than `max_messages` are
/// needed, the listing is cut off and the last message says how many items
/// were left out.
fn chunk_items(
    header: &str,
    items: &[String],
    sep: &str,
    budget: usize,
    max_messages: usize,
) -> Vec<String> {
    let budget = budget.saturating_sub(TRUNCATION_NOTE_RESERVE);
    let sep_len = sep.chars().count();

    // (text, items in it). A chunk starts with the header (first) or nothing.
    let mut chunks: Vec<(String, usize)> = Vec::new();
    let mut current = header.to_string();
    let mut current_len = current.chars().count();
    let mut count = 0;

    for item in items {
        // A single oversized item is shortened rather than overflowing.
        let item: String = if item.chars().count() > budget {
            let mut short: String = item.chars().take(budget.saturating_sub(1)).collect();
            short.push('…');
            short
        } else {
            item.clone()
        };
        let item_len = item.chars().count();
        let added = if count == 0 { 0 } else { sep_len } + item_len;

        if current_len + added > budget && count > 0 {
            chunks.push((std::mem::take(&mut current), count));
            current_len = 0;
            count = 0;
        }
        if count > 0 {
            current.push_str(sep);
            current_len += sep_len;
        }
        current.push_str(&item);
        current_len += item_len;
        count += 1;
    }
    chunks.push((current, count));

    if chunks.len() > max_messages {
        let omitted: usize = chunks[max_messages..].iter().map(|(_, n)| n).sum();
        chunks.truncate(max_messages);
        if let Some((last, _)) = chunks.last_mut() {
            last.push_str(&format!("\n…and {omitted} more not shown."));
        }
    }
    chunks.into_iter().map(|(text, _)| text).collect()
}

fn format_show_list(shows: &[(String, i64)]) -> Vec<String> {
    if shows.is_empty() {
        return vec!["There are no shows yet. Add one with !shows -add <title>.".to_string()];
    }
    let items: Vec<String> = shows
        .iter()
        .map(|(title, n)| format!("{title} ({n})"))
        .collect();
    chunk_items(
        &format!(
            "Shows ({}) with episode counts. Use !episodes -show <show> to see a show's episodes:\n",
            shows.len()
        ),
        &items,
        ", ",
        MESSAGE_BUDGET,
        MAX_MESSAGES,
    )
}

fn format_episode_list(show_title: &str, episodes: &[(String, String)]) -> Vec<String> {
    let show_arg = command_arg(show_title);
    if episodes.is_empty() {
        return vec![format!(
            "{show_title} has no episodes yet. Add one with !episodes -add -show {show_arg} -episode <ep> [title]."
        )];
    }
    let items: Vec<String> = episodes
        .iter()
        .map(|(code, title)| {
            if title.is_empty() {
                code.clone()
            } else {
                format!("{code} - {title}")
            }
        })
        .collect();
    chunk_items(
        &format!(
            "{show_title}: {} episode{} (use a code below in !quote -add -show {show_arg} -episode <code> <quote text>)\n",
            episodes.len(),
            if episodes.len() == 1 { "" } else { "s" }
        ),
        &items,
        "\n",
        MESSAGE_BUDGET,
        MAX_MESSAGES,
    )
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// The outcome of resolving a user's show reference to a row.
#[derive(Debug, PartialEq, Eq)]
pub enum ShowLookup {
    Found {
        id: i64,
        title: String,
    },
    NotFound,
    /// More than one show matched; holds (up to a limit) their titles.
    Ambiguous(Vec<String>),
}

/// Escape `\`, `%` and `_` so user text is matched literally inside a LIKE pattern.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `"mystery sci"` -> `"%mystery%sci%"`: the words, in order, with anything between.
fn like_pattern(query: &str) -> String {
    let words: Vec<String> = query.split_whitespace().map(escape_like).collect();
    format!("%{}%", words.join("%"))
}

/// Find the show a user means.
///
/// An exact title match wins outright. Otherwise the text is treated as words
/// that must appear in order in the title; exactly one such show is a match,
/// and several is ambiguous. Show titles are short codes, and one can contain
/// another (`MST3K` and `MST3K60m`), so picking an arbitrary partial match would
/// silently send quotes and episodes to the wrong show.
pub(crate) fn resolve_show(conn: &mut PooledConn, query: &str) -> mysql::Result<ShowLookup> {
    let query = query.trim();

    let exact: Vec<(i64, String)> = conn.exec(
        "SELECT show_id, show_title FROM masterlist_shows \
         WHERE show_title = ? ORDER BY show_id LIMIT 1",
        (query,),
    )?;
    if let Some((id, title)) = exact.into_iter().next() {
        return Ok(ShowLookup::Found { id, title });
    }

    if query.is_empty() {
        return Ok(ShowLookup::NotFound);
    }

    let mut partial: Vec<(i64, String)> = conn.exec(
        "SELECT show_id, show_title FROM masterlist_shows \
         WHERE show_title LIKE ? ORDER BY show_title LIMIT ?",
        (like_pattern(query), MAX_AMBIGUOUS_SHOWN + 1),
    )?;
    Ok(match partial.len() {
        0 => ShowLookup::NotFound,
        1 => {
            let (id, title) = partial.remove(0);
            ShowLookup::Found { id, title }
        }
        _ => ShowLookup::Ambiguous(partial.into_iter().map(|(_, t)| t).collect()),
    })
}

/// Why a show reference could not be used, as a message for the user.
pub(crate) fn show_problem_message(query: &str, lookup: &ShowLookup) -> Option<String> {
    match lookup {
        ShowLookup::Found { .. } => None,
        ShowLookup::NotFound => Some(format!(
            "No show found matching '{query}'. Use !shows to see the valid shows, or !shows -add <title> to add it."
        )),
        ShowLookup::Ambiguous(titles) => {
            let shown = titles.len().min(MAX_AMBIGUOUS_SHOWN);
            let mut list = titles[..shown].join(", ");
            if titles.len() > shown {
                list.push_str(", …");
            }
            Some(format!(
                "'{query}' matches several shows ({list}). Please be more specific."
            ))
        }
    }
}

/// Run a parsed command against the database and return the reply messages.
fn run(conn: &mut PooledConn, command: Command) -> mysql::Result<Vec<String>> {
    match command {
        Command::Invalid(message) => Ok(vec![message]),

        Command::ListShows => {
            let shows: Vec<(String, i64)> = conn.query(
                "SELECT s.show_title, COUNT(e.episode_id) FROM masterlist_shows s \
                 LEFT JOIN masterlist_episodes e ON e.show_id = s.show_id \
                 GROUP BY s.show_id, s.show_title ORDER BY s.show_title",
            )?;
            Ok(format_show_list(&shows))
        }

        Command::AddShow { title } => {
            // One atomic statement: no row is inserted if the title already
            // exists (case-insensitively), even with a concurrent add. The
            // tables have no unique key, so a separate check could race.
            conn.exec_drop(
                "INSERT INTO masterlist_shows (show_title) \
                 SELECT ? FROM DUAL \
                 WHERE NOT EXISTS (SELECT 1 FROM masterlist_shows WHERE show_title = ?)",
                (&title, &title),
            )?;
            if conn.affected_rows() == 0 {
                Ok(vec![format!(
                    "A show named '{title}' already exists. See its episodes with !episodes -show {}.",
                    command_arg(&title)
                )])
            } else {
                Ok(vec![format!(
                    "Added show '{title}' (id {}). Next, add its episodes: !episodes -add -show {} -episode <ep> [title]",
                    conn.last_insert_id(),
                    command_arg(&title)
                )])
            }
        }

        Command::ListEpisodes { show } => {
            let lookup = resolve_show(conn, &show)?;
            if let Some(problem) = show_problem_message(&show, &lookup) {
                return Ok(vec![problem]);
            }
            let ShowLookup::Found { id, title } = lookup else {
                unreachable!("show_problem_message returns None only for Found")
            };
            let episodes: Vec<(String, String)> = conn.exec(
                "SELECT show_ep, title FROM masterlist_episodes \
                 WHERE show_id = ? ORDER BY sort, show_ep, episode_id",
                (id,),
            )?;
            Ok(format_episode_list(&title, &episodes))
        }

        Command::CheckEpisode { show, episode } => {
            let lookup = resolve_show(conn, &show)?;
            if let Some(problem) = show_problem_message(&show, &lookup) {
                return Ok(vec![problem]);
            }
            let ShowLookup::Found { id, title } = lookup else {
                unreachable!("show_problem_message returns None only for Found")
            };
            let found: Vec<(String, String)> = conn.exec(
                "SELECT show_ep, title FROM masterlist_episodes \
                 WHERE show_id = ? AND show_ep = ? ORDER BY episode_id",
                (id, &episode),
            )?;
            if found.is_empty() {
                let show_arg = command_arg(&title);
                return Ok(vec![format!(
                    "{title} has no episode '{episode}'. See its episode codes with !episodes -show {show_arg}, \
                     or add it with !episodes -add -show {show_arg} -episode {} [title]",
                    command_arg(&episode)
                )]);
            }
            let described: Vec<String> = found
                .iter()
                .map(|(code, ep_title)| {
                    if ep_title.is_empty() {
                        format!("{title} {code} exists (no title)")
                    } else {
                        format!("{title} {code} exists: {ep_title}")
                    }
                })
                .collect();
            Ok(vec![described.join("\n")])
        }

        Command::AddEpisode {
            show,
            episode,
            title: ep_title,
        } => {
            let lookup = resolve_show(conn, &show)?;
            if let Some(problem) = show_problem_message(&show, &lookup) {
                return Ok(vec![problem]);
            }
            let ShowLookup::Found { id, title } = lookup else {
                unreachable!("show_problem_message returns None only for Found")
            };
            let show_arg = command_arg(&title);
            let quote_hint = format!(
                "!quote -add -show {show_arg} -episode {} <quote text>",
                command_arg(&episode)
            );
            // Atomic, like AddShow: (show_id, show_ep) is not a unique key.
            conn.exec_drop(
                "INSERT INTO masterlist_episodes (show_id, show_ep, title) \
                 SELECT ?, ?, ? FROM DUAL \
                 WHERE NOT EXISTS (SELECT 1 FROM masterlist_episodes WHERE show_id = ? AND show_ep = ?)",
                (id, &episode, &ep_title, id, &episode),
            )?;
            if conn.affected_rows() == 0 {
                Ok(vec![format!(
                    "{title} already has an episode '{episode}'. You can add a quote to it with {quote_hint}"
                )])
            } else if ep_title.is_empty() {
                Ok(vec![format!(
                    "Added {title} episode {episode} (no title). Now add a quote with {quote_hint}"
                )])
            } else {
                Ok(vec![format!(
                    "Added {title} episode {episode}: {ep_title}\nNow add a quote with {quote_hint}"
                )])
            }
        }
    }
}

/// MySQL error 1366: a value has characters the column's charset can't store.
const ER_TRUNCATED_WRONG_VALUE_FOR_FIELD: u16 = 1366;
/// MySQL error 1267: comparing text to a column whose charset can't represent it.
const ER_CANT_AGGREGATE_2COLLATIONS: u16 = 1267;

/// The masterlist tables are utf8mb4 (see migrations/2026-11-utf8mb4.sql), so
/// any valid text fits. These errors only appear if the columns can't hold the
/// text, e.g. a server whose tables were not converted. The server runs in strict
/// mode, so nothing is silently altered: a store fails with 1366 and a comparison
/// with 1267, and both mean the same to the user.
pub(crate) fn describe_db_error(e: &mysql::Error) -> String {
    match e {
        mysql::Error::MySqlError(me)
            if matches!(
                me.code,
                ER_TRUNCATED_WRONG_VALUE_FOR_FIELD | ER_CANT_AGGREGATE_2COLLATIONS
            ) =>
        {
            "That text has characters the quote database can't store.".to_string()
        }
        _ => "The quote database returned an error.".to_string(),
    }
}

/// Blocking: run a command and produce the messages to send.
pub fn execute(pool: &Option<Pool>, command: Command) -> Vec<String> {
    // Malformed input never needs the database.
    if let Command::Invalid(message) = command {
        return vec![message];
    }
    let Some(pool) = pool else {
        error!("MySQL pool is None when handling a shows/episodes command");
        return vec!["MySQL database is not configured.".to_string()];
    };
    let mut conn = match pool.get_conn() {
        Ok(conn) => conn,
        Err(e) => {
            error!("Failed to get MySQL connection for shows/episodes: {:?}", e);
            return vec!["Failed to connect to the quote database.".to_string()];
        }
    };
    match run(&mut conn, command) {
        Ok(messages) => messages,
        Err(e) => {
            error!("Database error handling shows/episodes command: {:?}", e);
            vec![describe_db_error(&e)]
        }
    }
}

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

/// Run a command and reply in Discord.
///
/// Replies suppress all mentions: show and episode titles are user-supplied
/// and stored permanently, so a title like `@everyone` must never ping anyone
/// when it is listed later.
pub async fn handle(
    db: &DatabaseManager,
    http: &Http,
    msg: &Message,
    command: Command,
) -> Result<()> {
    info!("Masterlist command: {:?}", command);
    let pool = db.pool.clone();
    let replies = tokio::task::spawn_blocking(move || execute(&pool, command))
        .await
        .unwrap_or_else(|_| vec!["The database task failed unexpectedly.".to_string()]);

    for (i, text) in replies.iter().enumerate() {
        let mut builder = CreateMessage::new().content(text);
        builder = if i == 0 {
            // Reply to the user (and ping them, as other bot replies do), but no one else.
            builder
                .reference_message(msg)
                .allowed_mentions(CreateAllowedMentions::new().replied_user(true))
        } else {
            builder.allowed_mentions(CreateAllowedMentions::new())
        };
        msg.channel_id.send_message(http, builder).await?;
    }
    Ok(())
}

/// Shared helpers for the database integration tests.
///
/// Those tests are `#[ignore]`d. To run them, point `CROW_TEST_MYSQL_URL` at a
/// scratch MySQL/MariaDB database whose name starts with `crow_test`, e.g.
///
/// ```text
/// CROW_TEST_MYSQL_URL=mysql://root@127.0.0.1:33306/crow_test \
///     cargo test -- --ignored --test-threads=1
/// ```
///
/// The tests DROP and recreate the `masterlist_*` tables, so they refuse to run
/// against any other database name.
#[cfg(test)]
pub(crate) mod test_support {
    use mysql::{prelude::*, Pool};
    use std::sync::{Mutex, MutexGuard};

    static DB_LOCK: Mutex<()> = Mutex::new(());

    /// A pool on a freshly reset scratch database. Holds a lock so integration
    /// tests that share the tables never run at the same time.
    pub struct TestDb {
        pub pool: Pool,
        _guard: MutexGuard<'static, ()>,
    }

    /// The production table definitions (MyISAM, utf8mb4 since migrations/2026-11-utf8mb4.sql, no unique key on
    /// `(show_id, show_ep)`), seeded with a few rows.
    pub fn fresh_db() -> TestDb {
        let guard = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let url = std::env::var("CROW_TEST_MYSQL_URL")
            .expect("set CROW_TEST_MYSQL_URL to a scratch database named crow_test* to run this");
        let pool = Pool::new(url.as_str()).expect("connect to test database");
        let mut conn = pool.get_conn().expect("get connection");

        let db: Option<String> = conn
            .query_first("SELECT DATABASE()")
            .expect("query database name")
            .flatten();
        let db = db.unwrap_or_default();
        assert!(
            db.starts_with("crow_test"),
            "refusing to reset tables in database {db:?}: its name must start with crow_test"
        );

        let statements = [
            "DROP TABLE IF EXISTS masterlist_quotes, masterlist_episodes, masterlist_shows",
            "CREATE TABLE masterlist_shows (
                show_id int(11) NOT NULL AUTO_INCREMENT,
                show_title varchar(255) NOT NULL DEFAULT '',
                show_desc mediumtext DEFAULT NULL,
                air_dates varchar(255) DEFAULT NULL,
                show_shorts tinyint(1) DEFAULT NULL,
                active tinyint(1) DEFAULT NULL,
                PRIMARY KEY (show_id)
            ) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            "CREATE TABLE masterlist_episodes (
                episode_id int(8) NOT NULL AUTO_INCREMENT,
                show_id int(4) unsigned NOT NULL DEFAULT 0,
                sort tinyint(3) NOT NULL DEFAULT 0,
                show_ep varchar(10) NOT NULL DEFAULT '',
                title varchar(255) NOT NULL DEFAULT '',
                description text DEFAULT NULL,
                buyable varchar(255) NOT NULL DEFAULT '0',
                current_file_id int(10) DEFAULT NULL,
                current_label_id int(10) DEFAULT NULL,
                current_insert_id int(10) DEFAULT NULL,
                current_dvd_id int(10) unsigned DEFAULT NULL,
                list_in_ap tinyint(3) unsigned NOT NULL DEFAULT 0,
                PRIMARY KEY (episode_id),
                KEY masterlist_episodes_show_id_show_ep (show_id, show_ep)
            ) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            "CREATE TABLE masterlist_quotes (
                quote_id int(10) unsigned NOT NULL AUTO_INCREMENT,
                show_id int(4) unsigned NOT NULL DEFAULT 0,
                show_ep varchar(10) NOT NULL DEFAULT '',
                pn_uid int(5) unsigned NOT NULL DEFAULT 0,
                time timestamp NOT NULL DEFAULT current_timestamp() ON UPDATE current_timestamp(),
                quote text NOT NULL,
                ap_quote_id int(10) unsigned DEFAULT NULL,
                PRIMARY KEY (quote_id)
            ) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            // MST3K is a prefix of MST3K60m, like production.
            "INSERT INTO masterlist_shows (show_id, show_title) VALUES
                (1, 'MST3K'), (2, 'MST3K60m'), (3, 'SG'), (4, 'ATHF')",
            "INSERT INTO masterlist_episodes (show_id, sort, show_ep, title) VALUES
                (1, 0, '0102', 'Robot Monster'),
                (1, 0, '0101', 'The Crawling Eye'),
                (1, 1, 'S01', ''),
                (1, -1, 'K00', 'The Green Slime'),
                (4, 0, '321', 'The Dressing'),
                (4, 0, '321', 'The Dressing Again')",
        ];
        for sql in statements {
            conn.query_drop(sql)
                .unwrap_or_else(|e| panic!("setup failed: {e}\n{sql}"));
        }
        TestDb {
            pool,
            _guard: guard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &[&str]) -> Vec<String> {
        s.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn args_text_strips_the_command_word() {
        assert_eq!(args_text("!shows"), "");
        assert_eq!(args_text("!shows   -add  Foo Bar"), "-add  Foo Bar");
        assert_eq!(args_text("!episodes -show x"), "-show x");
        assert_eq!(args_text("  !episodes\t-show x"), "-show x");
    }

    #[test]
    fn shows_parsing() {
        assert_eq!(parse_shows(&[]), Command::ListShows);
        assert_eq!(
            parse_shows(&toks(&["-add", "Mystery", "Science"])),
            Command::AddShow {
                title: "Mystery Science".into()
            }
        );
        assert_eq!(
            parse_shows(&toks(&["-ADD", "Foo"])),
            Command::AddShow {
                title: "Foo".into()
            }
        );
        assert!(matches!(parse_shows(&toks(&["-add"])), Command::Invalid(_)));
        assert!(matches!(
            parse_shows(&toks(&["mst3k"])),
            Command::Invalid(_)
        ));
    }

    #[test]
    fn episodes_parsing_list_check_add() {
        assert_eq!(
            parse_episodes(&toks(&["-show", "mst3k"])),
            Command::ListEpisodes {
                show: "mst3k".into()
            }
        );
        assert_eq!(
            parse_episodes(&toks(&["-show", "mst3k", "-episode", "0101"])),
            Command::CheckEpisode {
                show: "mst3k".into(),
                episode: "0101".into()
            }
        );
        assert_eq!(
            parse_episodes(&toks(&["-show", "x", "-ep", "A1"])),
            Command::CheckEpisode {
                show: "x".into(),
                episode: "A1".into()
            }
        );
        assert_eq!(
            parse_episodes(&toks(&[
                "-add", "-show", "Foo Show", "-episode", "101", "The", "Pilot"
            ])),
            Command::AddEpisode {
                show: "Foo Show".into(),
                episode: "101".into(),
                title: "The Pilot".into()
            }
        );
    }

    #[test]
    fn episode_title_is_optional_when_adding() {
        assert_eq!(
            parse_episodes(&toks(&["-add", "-show", "x", "-episode", "K99"])),
            Command::AddEpisode {
                show: "x".into(),
                episode: "K99".into(),
                title: String::new()
            }
        );
    }

    #[test]
    fn episodes_parsing_rejects_bad_input() {
        // No show.
        assert!(matches!(parse_episodes(&[]), Command::Invalid(_)));
        assert!(matches!(
            parse_episodes(&toks(&["-add", "-episode", "1"])),
            Command::Invalid(_)
        ));
        // Add without an episode code.
        assert!(matches!(
            parse_episodes(&toks(&["-add", "-show", "x", "Title"])),
            Command::Invalid(_)
        ));
        // A flag missing its value.
        assert!(matches!(
            parse_episodes(&toks(&["-show"])),
            Command::Invalid(_)
        ));
        // Stray text without -add must not be silently ignored.
        assert!(matches!(
            parse_episodes(&toks(&["-show", "x", "-episode", "1", "oops"])),
            Command::Invalid(_)
        ));
        // Code too long / containing spaces.
        assert!(matches!(
            parse_episodes(&toks(&["-add", "-show", "x", "-episode", "12345678901"])),
            Command::Invalid(_)
        ));
        assert!(matches!(
            parse_episodes(&toks(&["-add", "-show", "x", "-episode", "S1 E1"])),
            Command::Invalid(_)
        ));
    }

    #[test]
    fn validation_limits() {
        assert!(validate_show_title(&"a".repeat(255)).is_ok());
        assert!(validate_show_title(&"a".repeat(256)).is_err());
        assert!(validate_show_title("  ").is_err());
        assert!(validate_show_title("two\nlines").is_err());
        assert!(validate_episode_code(&"a".repeat(10)).is_ok());
        assert!(validate_episode_code(&"a".repeat(11)).is_err());
        assert!(validate_episode_title("").is_ok());
        assert!(validate_episode_title(&"é".repeat(255)).is_ok());
        assert!(validate_episode_title(&"é".repeat(256)).is_err());
    }

    #[test]
    fn like_pattern_matches_words_in_order_and_escapes() {
        assert_eq!(like_pattern("mst3k"), "%mst3k%");
        assert_eq!(like_pattern("  mystery   sci "), "%mystery%sci%");
        // User-typed wildcards must be literal.
        assert_eq!(like_pattern("50%_off"), "%50\\%\\_off%");
        assert_eq!(like_pattern("a\\b"), "%a\\\\b%");
    }

    #[test]
    fn chunking_splits_on_budget_and_keeps_header_first() {
        let items: Vec<String> = (0..10).map(|i| format!("item{i}")).collect();
        // Each message may hold roughly 3 items; the note reserve is subtracted.
        let budget = TRUNCATION_NOTE_RESERVE + 25;
        let chunks = chunk_items("H: ", &items, ", ", budget, 100);
        assert!(chunks.len() > 1);
        assert!(chunks[0].starts_with("H: item0"));
        for c in &chunks {
            assert!(c.chars().count() <= budget - TRUNCATION_NOTE_RESERVE, "{c}");
        }
        // Every item appears exactly once, in order.
        let joined = chunks.join(", ");
        for i in 0..10 {
            assert_eq!(joined.matches(&format!("item{i}")).count(), 1);
        }
    }

    #[test]
    fn chunking_truncates_with_an_accurate_count() {
        let items: Vec<String> = (0..50).map(|i| format!("entry-{i:02}")).collect();
        let budget = TRUNCATION_NOTE_RESERVE + 30;
        let all = chunk_items("", &items, "\n", budget, 1000);
        let capped = chunk_items("", &items, "\n", budget, 2);
        assert_eq!(capped.len(), 2);
        let shown: usize = capped.iter().map(|c| c.matches("entry-").count()).sum();
        let omitted: usize = capped[1]
            .rsplit("…and ")
            .next()
            .and_then(|s| s.split(' ').next())
            .and_then(|n| n.parse().ok())
            .expect("omitted count in note");
        assert_eq!(shown + omitted, 50);
        assert!(all.len() > 2);
        // Capped messages still respect the Discord limit with the note added.
        for c in &capped {
            assert!(c.chars().count() <= budget);
        }
    }

    #[test]
    fn chunking_shortens_one_oversized_item() {
        let chunks = chunk_items("", &["x".repeat(500)], ", ", 100, 5);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].chars().count() <= 100);
        assert!(chunks[0].ends_with('…'));
    }

    #[test]
    fn real_sized_listings_fit_the_discord_limit() {
        // The biggest show today has ~250 episodes with titles up to 79 chars.
        let eps: Vec<(String, String)> = (0..250)
            .map(|i| (format!("{:04}", i), "T".repeat(79)))
            .collect();
        for message in format_episode_list("MST3K", &eps) {
            assert!(message.chars().count() <= 2000);
        }
        let shows: Vec<(String, i64)> = (0..200).map(|i| (format!("SHOW{i}"), 100)).collect();
        for message in format_show_list(&shows) {
            assert!(message.chars().count() <= 2000);
        }
    }

    #[test]
    fn episode_list_header_pluralizes() {
        let one = format_episode_list("X", &[("1".into(), "t".into())]);
        assert!(one[0].starts_with("X: 1 episode ("), "{}", one[0]);
        let two = format_episode_list("X", &[("1".into(), "t".into()), ("2".into(), "u".into())]);
        assert!(two[0].starts_with("X: 2 episodes ("), "{}", two[0]);
    }

    #[test]
    fn empty_listings_have_helpful_messages() {
        assert_eq!(format_show_list(&[]).len(), 1);
        assert!(format_episode_list("X", &[])[0].contains("no episodes"));
    }

    #[test]
    fn command_arg_quotes_only_when_needed() {
        assert_eq!(command_arg("MST3K"), "MST3K");
        assert_eq!(command_arg("Space Ghost"), "\"Space Ghost\"");
    }

    #[test]
    fn help_topic_words() {
        for t in [
            "quotes", "QUOTE", "shows", "episodes", "Episode", "edit", "Undo", "history",
        ] {
            assert!(is_quotes_help_topic(t), "{t}");
        }
        assert!(!is_quotes_help_topic("imagine"));
        assert!(!is_quotes_help_topic(""));
    }

    #[test]
    fn walkthrough_fits_in_one_discord_message() {
        assert!(QUOTES_HELP.chars().count() <= 2000);
    }

    #[test]
    fn mistakes_get_a_specific_hint() {
        // Search text where there is none.
        let Command::Invalid(m) = parse_shows(&toks(&["mst3k"])) else {
            panic!()
        };
        assert!(
            m.contains("takes no search text") && m.contains("!shows -add"),
            "{m}"
        );
        // Forgot -show; the suggestion must be a command that would work.
        let Command::Invalid(m) = parse_episodes(&toks(&["mst3k"])) else {
            panic!()
        };
        assert!(m.contains("Did you mean !episodes -show mst3k?"), "{m}");
        let Command::Invalid(m) = parse_episodes(&toks(&["Space", "Ghost"])) else {
            panic!()
        };
        assert!(m.contains("!episodes -show \"Space Ghost\"?"), "{m}");
        // No guess when it would be wrong.
        let Command::Invalid(m) = parse_episodes(&toks(&["-add", "-episode", "1", "T"])) else {
            panic!()
        };
        assert!(!m.contains("Did you mean"), "{m}");
        // Every error includes usage with an example and points at the walkthrough.
        for c in [
            parse_shows(&toks(&["x"])),
            parse_episodes(&[]),
            parse_episodes(&toks(&["-show"])),
        ] {
            let Command::Invalid(m) = c else { panic!() };
            assert!(m.contains("Usage:") && m.contains("!help quotes"), "{m}");
        }
    }

    #[test]
    fn show_problem_messages() {
        assert_eq!(
            show_problem_message(
                "x",
                &ShowLookup::Found {
                    id: 1,
                    title: "X".into()
                }
            ),
            None
        );
        assert!(show_problem_message("x", &ShowLookup::NotFound)
            .unwrap()
            .contains("!shows -add"));
        let amb = show_problem_message("mst", &ShowLookup::Ambiguous(vec!["A".into(), "B".into()]))
            .unwrap();
        assert!(amb.contains("A, B") && amb.contains("more specific"));
    }

    // ---- Database integration tests (see test_support for how to run) ----

    use super::test_support::fresh_db;

    fn run_cmd(pool: &Pool, command: Command) -> String {
        execute(&Some(pool.clone()), command).join("\n")
    }

    fn show_count(pool: &Pool, title: &str) -> i64 {
        let mut conn = pool.get_conn().unwrap();
        conn.exec_first(
            "SELECT COUNT(*) FROM masterlist_shows WHERE show_title = ?",
            (title,),
        )
        .unwrap()
        .unwrap()
    }

    fn episode_count(pool: &Pool, show_id: i64, code: &str) -> i64 {
        let mut conn = pool.get_conn().unwrap();
        conn.exec_first(
            "SELECT COUNT(*) FROM masterlist_episodes WHERE show_id = ? AND show_ep = ?",
            (show_id, code),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_show_resolution() {
        let db = fresh_db();
        let mut conn = db.pool.get_conn().unwrap();
        let found = |r: ShowLookup| match r {
            ShowLookup::Found { id, title } => (id, title),
            other => panic!("expected Found, got {other:?}"),
        };

        // Exact title wins even though "mst3k" is also a substring of MST3K60m,
        // and regardless of case.
        assert_eq!(
            found(resolve_show(&mut conn, "MST3K").unwrap()),
            (1, "MST3K".into())
        );
        assert_eq!(
            found(resolve_show(&mut conn, "mst3k").unwrap()),
            (1, "MST3K".into())
        );
        assert_eq!(
            found(resolve_show(&mut conn, " MST3K60m ").unwrap()),
            (2, "MST3K60m".into())
        );
        // A unique partial match is accepted.
        assert_eq!(
            found(resolve_show(&mut conn, "60m").unwrap()),
            (2, "MST3K60m".into())
        );
        assert_eq!(
            found(resolve_show(&mut conn, "ath").unwrap()),
            (4, "ATHF".into())
        );
        // A partial match shared by several shows is ambiguous, not arbitrary.
        assert_eq!(
            resolve_show(&mut conn, "mst3").unwrap(),
            ShowLookup::Ambiguous(vec!["MST3K".into(), "MST3K60m".into()])
        );
        assert_eq!(
            resolve_show(&mut conn, "zzz").unwrap(),
            ShowLookup::NotFound
        );
        assert_eq!(
            resolve_show(&mut conn, "   ").unwrap(),
            ShowLookup::NotFound
        );
        // User-typed wildcards are literal, not "match everything".
        assert_eq!(resolve_show(&mut conn, "%").unwrap(), ShowLookup::NotFound);
        assert_eq!(resolve_show(&mut conn, "_").unwrap(), ShowLookup::NotFound);
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_list_shows_with_counts() {
        let db = fresh_db();
        let out = run_cmd(&db.pool, Command::ListShows);
        assert_eq!(
            out,
            "Shows (4) with episode counts. Use !episodes -show <show> to see a show's episodes:\n\
             ATHF (2), MST3K (4), MST3K60m (0), SG (0)"
        );
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_add_show() {
        let db = fresh_db();
        let out = run_cmd(
            &db.pool,
            Command::AddShow {
                title: "Space Ghost".into(),
            },
        );
        assert!(out.starts_with("Added show 'Space Ghost' (id 5)."), "{out}");
        // The suggested next step must work for a name with spaces.
        assert!(
            out.contains("!episodes -add -show \"Space Ghost\" -episode <ep>"),
            "{out}"
        );
        assert_eq!(show_count(&db.pool, "Space Ghost"), 1);

        // Duplicate, including a different case, is refused and adds nothing.
        for dup in ["Space Ghost", "SPACE GHOST", "mst3k"] {
            let out = run_cmd(&db.pool, Command::AddShow { title: dup.into() });
            assert!(out.contains("already exists"), "{out}");
        }
        assert_eq!(show_count(&db.pool, "Space Ghost"), 1);
        assert_eq!(show_count(&db.pool, "MST3K"), 1);

        // A new show appears in the list and is immediately usable.
        let list = run_cmd(&db.pool, Command::ListShows);
        assert!(list.contains("Space Ghost (0)"), "{list}");
        let out = run_cmd(
            &db.pool,
            Command::ListEpisodes {
                show: "space ghost".into(),
            },
        );
        assert!(out.contains("no episodes yet"), "{out}");
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_list_and_check_episodes() {
        let db = fresh_db();

        // Ordered by section (sort) then code: K00, 0101, 0102, then S01.
        let out = run_cmd(
            &db.pool,
            Command::ListEpisodes {
                show: "mst3k".into(),
            },
        );
        assert_eq!(
            out,
            "MST3K: 4 episodes (use a code below in !quote -add -show MST3K -episode <code> <quote text>)\nK00 - The Green Slime\n0101 - The Crawling Eye\n0102 - Robot Monster\nS01"
        );

        let check = |show: &str, ep: &str| {
            run_cmd(
                &db.pool,
                Command::CheckEpisode {
                    show: show.into(),
                    episode: ep.into(),
                },
            )
        };
        assert_eq!(
            check("mst3k", "0101"),
            "MST3K 0101 exists: The Crawling Eye"
        );
        assert_eq!(check("mst3k", "k00"), "MST3K K00 exists: The Green Slime");
        assert_eq!(check("mst3k", "S01"), "MST3K S01 exists (no title)");
        let missing = check("mst3k", "9999");
        assert!(
            missing.starts_with("MST3K has no episode '9999'.")
                && missing.contains("!episodes -show MST3K")
                && missing.contains("!episodes -add -show MST3K -episode 9999 [title]"),
            "{missing}"
        );
        // Pre-existing duplicate rows are all reported.
        assert_eq!(
            check("athf", "321"),
            "ATHF 321 exists: The Dressing\nATHF 321 exists: The Dressing Again"
        );

        let out = run_cmd(
            &db.pool,
            Command::ListEpisodes {
                show: "mst3".into(),
            },
        );
        assert!(
            out.contains("matches several shows") && out.contains("MST3K60m"),
            "{out}"
        );
        let out = run_cmd(
            &db.pool,
            Command::CheckEpisode {
                show: "nope".into(),
                episode: "1".into(),
            },
        );
        assert!(out.contains("No show found"), "{out}");
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_add_episode() {
        let db = fresh_db();
        let add = |show: &str, ep: &str, title: &str| {
            run_cmd(
                &db.pool,
                Command::AddEpisode {
                    show: show.into(),
                    episode: ep.into(),
                    title: title.into(),
                },
            )
        };

        assert_eq!(
            add("mst3k", "1001", "Pilot"),
            "Added MST3K episode 1001: Pilot\nNow add a quote with !quote -add -show MST3K -episode 1001 <quote text>"
        );
        assert_eq!(episode_count(&db.pool, 1, "1001"), 1);
        // The stored row uses the table defaults for everything else.
        let mut conn = db.pool.get_conn().unwrap();
        let row: (i64, String, i64) = conn
            .exec_first(
                "SELECT sort, buyable, list_in_ap FROM masterlist_episodes WHERE show_id = 1 AND show_ep = '1001'",
                (),
            )
            .unwrap()
            .unwrap();
        assert_eq!(row, (0, "0".to_string(), 0));

        // No title is allowed.
        assert!(add("MST3K", "1002", "")
            .starts_with("Added MST3K episode 1002 (no title). Now add a quote with"));

        // Duplicates are refused, case-insensitively, and nothing is inserted.
        let dup = add("mst3k", "1001", "Other");
        assert!(
            dup.starts_with("MST3K already has an episode '1001'.") && dup.contains("!quote -add"),
            "{dup}"
        );
        assert!(add("mst3k", "k00", "Other").starts_with("MST3K already has an episode 'k00'."));
        assert_eq!(episode_count(&db.pool, 1, "1001"), 1);
        assert_eq!(episode_count(&db.pool, 1, "K00"), 1);

        // The same code under a different show is fine.
        assert!(add("sg", "1001", "Other show").starts_with("Added SG episode 1001: Other show"));

        // Unknown / ambiguous show.
        assert!(add("nope", "1", "x").contains("No show found"));
        assert!(add("mst3", "1", "x").contains("matches several shows"));
        assert_eq!(episode_count(&db.pool, 2, "1"), 0);

        // A new episode is visible to list and check.
        let out = run_cmd(
            &db.pool,
            Command::CheckEpisode {
                show: "mst3k".into(),
                episode: "1001".into(),
            },
        );
        assert_eq!(out, "MST3K 1001 exists: Pilot");

        // A show whose name has spaces gets hints that can be pasted back in.
        run_cmd(
            &db.pool,
            Command::AddShow {
                title: "Space Ghost".into(),
            },
        );
        let out = add("space ghost", "101", "Pilot");
        assert!(
            out.contains("!quote -add -show \"Space Ghost\" -episode 101 <quote text>"),
            "{out}"
        );
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_emoji_and_non_latin_text_round_trip() {
        let db = fresh_db();
        // Read the exact bytes back: a lossy store would turn the emoji into "?".
        let stored_title = |pool: &Pool, like: &str| -> Option<String> {
            let mut conn = pool.get_conn().unwrap();
            conn.exec_first(
                "SELECT show_title FROM masterlist_shows WHERE show_title LIKE ? LIMIT 1",
                (like,),
            )
            .unwrap()
        };

        for title in [
            "Smile \u{1F600}",
            "\u{65E5}\u{672C}\u{8A9E}",
            "Caf\u{E9} \u{2019}n\u{2019} Co",
        ] {
            let out = run_cmd(
                &db.pool,
                Command::AddShow {
                    title: title.into(),
                },
            );
            assert!(out.starts_with("Added show"), "{title}: {out}");
            assert_eq!(stored_title(&db.pool, title).as_deref(), Some(title));
        }

        let out = run_cmd(
            &db.pool,
            Command::AddEpisode {
                show: "sg".into(),
                episode: "1".into(),
                title: "Moon \u{1F315} landing".into(),
            },
        );
        assert!(out.starts_with("Added SG episode 1"), "{out}");
        let mut conn = db.pool.get_conn().unwrap();
        let title: Option<String> = conn
            .exec_first(
                "SELECT title FROM masterlist_episodes WHERE show_id = 3 AND show_ep = '1'",
                (),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("Moon \u{1F315} landing"));

        // Looking up a show by emoji is an ordinary miss, not a database error.
        let out = run_cmd(
            &db.pool,
            Command::ListEpisodes {
                show: "\u{1F600}\u{1F600}".into(),
            },
        );
        assert!(!out.contains("can't handle"), "{out}");
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_long_listing_is_capped_across_messages() {
        let db = fresh_db();
        let mut conn = db.pool.get_conn().unwrap();
        for i in 0..300 {
            conn.exec_drop(
                "INSERT INTO masterlist_episodes (show_id, show_ep, title) VALUES (3, ?, ?)",
                (format!("E{i:03}"), "x".repeat(60)),
            )
            .unwrap();
        }
        let messages = execute(
            &Some(db.pool.clone()),
            Command::ListEpisodes { show: "sg".into() },
        );
        assert_eq!(messages.len(), MAX_MESSAGES);
        assert!(messages.iter().all(|m| m.chars().count() <= 2000));
        assert!(messages[0].starts_with("SG: 300 episodes"));
        assert!(messages.last().unwrap().contains("more not shown"));
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_concurrent_adds_never_duplicate() {
        let db = fresh_db();
        // Many connections add the same show / episode at once. The tables have
        // no unique key, so only the single-statement insert prevents duplicates.
        for round in 0..5 {
            let show = format!("Race{round}");
            let results: Vec<String> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..16)
                    .map(|_| {
                        let pool = db.pool.clone();
                        let show = show.clone();
                        scope.spawn(move || run_cmd(&pool, Command::AddShow { title: show }))
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let added = results
                .iter()
                .filter(|r| r.starts_with("Added show"))
                .count();
            assert_eq!(added, 1, "exactly one add should win: {results:?}");
            assert_eq!(show_count(&db.pool, &show), 1);

            let code = format!("R{round}");
            let results: Vec<String> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..16)
                    .map(|_| {
                        let pool = db.pool.clone();
                        let show = show.clone();
                        let code = code.clone();
                        scope.spawn(move || {
                            run_cmd(
                                &pool,
                                Command::AddEpisode {
                                    show,
                                    episode: code,
                                    title: "t".into(),
                                },
                            )
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let added = results.iter().filter(|r| r.starts_with("Added ")).count();
            assert_eq!(added, 1, "exactly one add should win: {results:?}");
            let mut conn = db.pool.get_conn().unwrap();
            let id: i64 = conn
                .exec_first(
                    "SELECT show_id FROM masterlist_shows WHERE show_title = ?",
                    (&show,),
                )
                .unwrap()
                .unwrap();
            assert_eq!(episode_count(&db.pool, id, &code), 1);
        }
    }

    #[test]
    #[ignore = "needs CROW_TEST_MYSQL_URL (scratch database named crow_test*)"]
    fn db_unconfigured_and_unreachable() {
        assert_eq!(
            execute(&None, Command::ListShows),
            vec!["MySQL database is not configured.".to_string()]
        );
        // Malformed input is answered without needing the database at all.
        assert_eq!(
            execute(&None, Command::Invalid("bad".into())),
            vec!["bad".to_string()]
        );
    }
}
