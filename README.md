# Crow Discord Bot

A Discord bot that follows specific channels and responds to various triggers including commands, mentions, and keywords.

## Features

- Responds to commands starting with `!`
- Responds to direct mentions
- Responds to messages starting with the bot's name
- Detects and responds to specific keywords
- Performs Google searches via web scraping
- Generates AI responses using Google's Gemini API
- Stores message history in a SQLite database
- Automatically trims the database to prevent excessive growth
- **Can follow multiple channels simultaneously**

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

- `!hello` - Bot responds with "world!"
- `!help` - Bot displays a list of available commands
- `!fightcrime` - Bot generates a crime fighting duo using recent chat participants
- `!quote [search_term]` - Bot returns a random quote, optionally filtered by search term
- `!quote -show [show_name]` - Bot returns a random quote from a specific show
- `!quote -dud [username]` - Bot returns a random message previously sent by the specified user
- `!slogan [search_term]` - Bot returns a random advertising slogan, optionally filtered by search term

## Message History Database

The bot stores message history in a SQLite database to enable features like `!quote -dud` and to maintain persistence across bot restarts.

### Database Schema

The messages table has the following structure:
```sql
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    author TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp INTEGER NOT NULL
)
```

### Database Management

The bot automatically manages its message history database:

1. New messages are stored as they arrive
2. The database is periodically trimmed to keep only the most recent messages (up to the `MESSAGE_HISTORY_LIMIT`)
3. The trim interval can be configured with `DB_TRIM_INTERVAL_SECS` (defaults to 3600 seconds / 1 hour)

This ensures that the bot's memory usage remains stable over time while still maintaining enough history for features like `!quote -dud`.

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
