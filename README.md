# Crow Discord Bot

A Discord bot that follows specific channels and responds to various triggers including commands, mentions, and keywords.

## Features

- Responds to commands starting with `!`
- Responds to direct mentions
- Responds to messages starting with the bot's name
- Detects and responds to specific keywords
- Performs Google searches via web scraping
- Generates AI responses using Google's Gemini API with conversation context
- Stores message history in a SQLite database
- Automatically trims the database to prevent excessive growth
- **Can follow multiple channels simultaneously**

## Available Commands

- `!help` - Show this help message
- `!hello` - Say hello
- `!buzz` - Generate a corporate buzzword phrase
- `!fightcrime` - Generate a crime fighting duo
- `!lastseen [name]` - Find when a user was last active
- `!quote [search_term]` - Get a random quote
- `!quote -show [show_name]` - Get a random quote from a specific show
- `!quote -dud [username]` - Get a random message from a user (or random user if no username provided)
- `!slogan [search_term]` - Get a random advertising slogan
- `!frinkiac [search_term]` - Get a Simpsons screenshot from Frinkiac (or random if no term provided)
- `!morbotron [search_term]` - Get a Futurama screenshot from Morbotron (or random if no term provided)
- `!masterofallscience [search_term]` - Get a Rick and Morty screenshot from Master of All Science (or random if no term provided)

## Setup Instructions

1. Create a `CrowConfig.toml` file based on the `CrowConfig.toml.example` template
2. Add your Discord bot token to the `DISCORD_TOKEN` field
3. Configure channels to follow using one of these options:
   - Single channel: `FOLLOWED_CHANNEL_ID` or `FOLLOWED_CHANNEL_NAME`
   - Multiple channels: `FOLLOWED_CHANNEL_IDS` or `FOLLOWED_CHANNEL_NAMES` (comma-separated)
4. Optionally specify `FOLLOWED_SERVER_NAME` to limit channel search to a specific server
5. Set the bot's name with the `BOT_NAME` field (defaults to "Crow" if not specified)
6. Set the message history limit with the `MESSAGE_HISTORY_LIMIT` field (defaults to 10000)
7. Set how often to trim the database with `DB_TRIM_INTERVAL_SECS` (defaults to 3600 seconds / 1 hour)
8. Configure Gemini API rate limits with `GEMINI_RATE_LIMIT_MINUTE` and `GEMINI_RATE_LIMIT_DAY` fields
9. For database functionality, add MySQL credentials
10. To enable/disable Google search, set `GOOGLE_SEARCH_ENABLED` to "true" or "false" (defaults to "true")
11. For AI responses, add Gemini API key

## Available Commands

## Available Commands

- `!help` - Show this help message
- `!hello` - Say hello
- `!buzz` - Generate a corporate buzzword phrase
- `!fightcrime` - Generate a crime fighting duo
- `!lastseen [name]` - Find when a user was last active
- `!quote [search_term]` - Get a random quote
- `!quote -show [show_name]` - Get a random quote from a specific show
- `!quote -dud [username]` - Get a random message from a user (or random user if no username provided)
- `!slogan [search_term]` - Get a random advertising slogan
- `!frinkiac [search_term]` - Get a Simpsons screenshot from Frinkiac (or random if no term provided)
- `!morbotron [search_term]` - Get a Futurama screenshot from Morbotron (or random if no term provided)
- `!masterofallscience [search_term]` - Get a Rick and Morty screenshot from Master of All Science (or random if no term provided)

## Message History Database

The bot stores message history in a SQLite database to enable features like `!quote -dud`, provide context for AI responses, and maintain persistence across bot restarts.

### Enhanced Database Schema

The messages table has the following structure:
```sql
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    guild_id TEXT,
    author_id TEXT NOT NULL,
    author TEXT NOT NULL,
    display_name TEXT,
    content TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    referenced_message_id TEXT
)
```

This enhanced schema stores all necessary fields from Discord messages, allowing the bot to:
1. Reconstruct complete message objects from the database
2. Provide rich context for AI responses
3. Track message references and relationships
4. Support advanced message history features

### Database Management

The bot automatically manages its message history database:

1. New messages are stored as they arrive with all metadata
2. The database is periodically trimmed to keep only the most recent messages (up to the `MESSAGE_HISTORY_LIMIT`)
3. The trim interval can be configured with `DB_TRIM_INTERVAL_SECS` (defaults to 3600 seconds / 1 hour)
4. Existing databases are automatically migrated to the enhanced schema

This ensures that the bot's memory usage remains stable over time while still maintaining enough history for features like `!quote -dud` and context-aware AI responses.

## Configuration Options

The bot can be configured through the `CrowConfig.toml` file:

- `DISCORD_TOKEN` - Your Discord bot token
- `FOLLOWED_CHANNEL_ID` - ID of a single channel to follow
- `FOLLOWED_CHANNEL_NAME` - Name of a single channel to follow
- `FOLLOWED_CHANNEL_IDS` - Comma-separated list of channel IDs to follow
- `FOLLOWED_CHANNEL_NAMES` - Comma-separated list of channel names to follow
- `FOLLOWED_SERVER_NAME` - Name of the server to look for channels in
- `BOT_NAME` - Name of the bot (defaults to "Crow")
- `MESSAGE_HISTORY_LIMIT` - Maximum number of messages to store (defaults to 10000)
- `DB_TRIM_INTERVAL_SECS` - How often to trim the database (defaults to 3600 seconds)
- `GEMINI_RATE_LIMIT_MINUTE` - Maximum Gemini API calls per minute (defaults to 15)
- `GEMINI_RATE_LIMIT_DAY` - Maximum Gemini API calls per day (defaults to 1500)
- `GEMINI_API_KEY` - Your Gemini API key
- `GEMINI_API_ENDPOINT` - Custom Gemini API endpoint
- `GEMINI_PROMPT_WRAPPER` - Custom prompt wrapper for Gemini API calls
- `GOOGLE_SEARCH_ENABLED` - Enable or disable Google search feature (defaults to "true")
- `DB_HOST`, `DB_NAME`, `DB_USER`, `DB_PASSWORD` - MySQL database credentials
## Quote and Slogan Database Structure

The bot uses a MySQL database to store and retrieve quotes and slogans. Here's the expected database structure:

### Quote Database Tables

The quote system requires three related tables:

1. **masterlist_shows** - Contains information about TV shows
   ```sql
   CREATE TABLE masterlist_shows (
       show_id INT PRIMARY KEY,
       show_title VARCHAR(255) NOT NULL
   );
   ```

2. **masterlist_episodes** - Contains information about episodes
   ```sql
   CREATE TABLE masterlist_episodes (
       show_id INT,
       show_ep VARCHAR(10),
       title VARCHAR(255) NOT NULL,
       PRIMARY KEY (show_id, show_ep),
       FOREIGN KEY (show_id) REFERENCES masterlist_shows(show_id)
   );
   ```

3. **masterlist_quotes** - Contains the actual quotes
   ```sql
   CREATE TABLE masterlist_quotes (
       quote_id INT PRIMARY KEY AUTO_INCREMENT,
       show_id INT,
       show_ep VARCHAR(10),
       quote TEXT NOT NULL,
       FOREIGN KEY (show_id, show_ep) REFERENCES masterlist_episodes(show_id, show_ep)
   );
   ```

### Slogan Database Table

The slogan system uses a single table:

```sql
CREATE TABLE nuke_quotes (
    pn_id INT PRIMARY KEY AUTO_INCREMENT,
    pn_quote TEXT NOT NULL
);
```

### User Message History

The bot maintains a comprehensive SQLite database to store user message history with all Discord metadata:

```sql
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    guild_id TEXT,
    author_id TEXT NOT NULL,
    author TEXT NOT NULL,
    display_name TEXT,
    content TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    referenced_message_id TEXT
);
```

This enhanced schema allows the bot to:
1. Store complete message objects with all Discord metadata
2. Provide rich context for AI responses
3. Support the `!quote -dud` command to retrieve random messages from users
4. Maintain conversation threads and references

The database is automatically migrated from older versions, preserving existing message history.

## Display Name Handling

The bot uses a sophisticated approach to determine the best display name for users:

1. **Server Nickname** - First priority, if the user has set a nickname in the server
2. **Global Display Name** - Second priority, if the user has set a global display name
3. **Username** - Last resort, if no other display name is available

This ensures that users are addressed by their preferred name in the server context, improving the personalization of the bot's responses.

The display name is used in various features:
- When addressing users in AI responses
- When storing messages in the database for `!quote -dud` command
- When generating crime fighting duos
- When responding to direct mentions or messages starting with the bot's name
## AI Response Feature

When the bot is directly mentioned in a message or when a message starts with the bot's name, it will:
1. If `thinking_message` is set to a non-empty string (and not "[none]"):
   - Send a "thinking" message (configurable via `THINKING_MESSAGE` in config)
   - Send the content to Google's Gemini API with conversation context
   - Apply a realistic typing delay based on response length (0.5 seconds per word)
   - Edit the "thinking" message with the AI-generated response
2. If `thinking_message` is empty or set to "[none]":
   - Send the content directly to Google's Gemini API with conversation context
   - Apply a realistic typing delay based on response length (0.5 seconds per word)
   - Post the AI-generated response without showing a "thinking" message first

The prompt sent to Gemini can be customized by setting the `GEMINI_PROMPT_WRAPPER` in your `CrowConfig.toml` file. The wrapper should include placeholders:
- `{message}` - The user's message
- `{bot_name}` - The bot's name
- `{user}` - The user's display name
- `{context}` - Recent conversation history (last 5 messages)

You can also configure which Gemini model to use by setting the `GEMINI_API_ENDPOINT` in your `CrowConfig.toml` file. This allows you to switch between different models like gemini-1.0-pro, gemini-1.5-pro, or gemini-1.5-flash.

## Conversation Context

The bot now includes conversation context when making API calls to Gemini. This means:

1. The bot retrieves the last 5 messages from the conversation history
2. These messages are included in the prompt sent to Gemini
3. Gemini can generate more contextually relevant responses based on the conversation flow
4. The bot appears more coherent and can maintain conversation threads

This feature makes the bot feel more natural in conversations and helps it remember what was previously discussed.
