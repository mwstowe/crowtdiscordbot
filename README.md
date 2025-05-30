# Crow Discord Bot

A Discord bot that follows specific channels and responds to various triggers including commands, mentions, and keywords.

## Discord Bot Setup

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Click "New Application" and give your bot a name
3. Go to the "Bot" section in the left sidebar
4. Click "Add Bot" to create a bot user
5. Under the bot's username, find and copy your bot token (you'll need this for `DISCORD_TOKEN` in config)
6. Enable the "Message Content Intent" under "Privileged Gateway Intents"
7. Go to the "OAuth2" section in the left sidebar
8. Under "Scopes", select:
   - `bot`
   - `messages.read`
   - `applications.commands`
9. Under "Bot Permissions", select the permissions your bot needs:
   - Read Messages/View Channels
   - Send Messages
   - Read Message History
   - Add Reactions
   - View Channel
   - Send Messages in Threads
10. Copy the generated OAuth2 URL at the bottom of the "Scopes" section
11. Open this URL in a browser to invite the bot to your server
    - You must have "Manage Server" permission in the Discord server

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
- Maintains channel-specific conversation contexts
- Configurable random interjections with separate probability controls

## Setup Instructions

1. Create a `CrowConfig.toml` file based on the `CrowConfig.toml.example` template
2. Add your Discord bot token to the `DISCORD_TOKEN` field
   - Privileged Gateway Intents: Message Content Intent
   - Scopes: `bot`, `messages.read`, `applications.commands`
3. Configure channels to follow using one of these options:
   - Single channel: `FOLLOWED_CHANNEL_ID` or `FOLLOWED_CHANNEL_NAME`
   - Multiple channels: `FOLLOWED_CHANNEL_IDS` or `FOLLOWED_CHANNEL_NAMES` (comma-separated)
4. Optionally specify `FOLLOWED_SERVER_NAME` to limit channel search to a specific server
5. Set the bot's name with the `BOT_NAME` field (defaults to "Crow" if not specified)
6. Set the message history limit with the `MESSAGE_HISTORY_LIMIT` field (defaults to 10000)
7. Set how often to trim the database with `DB_TRIM_INTERVAL_SECS` (defaults to 3600 seconds / 1 hour)
8. Configure Gemini API rate limits with `GEMINI_RATE_LIMIT_MINUTE` and `GEMINI_RATE_LIMIT_DAY` fields
9. Configure the number of context messages with `GEMINI_CONTEXT_MESSAGES` (defaults to 5)
10. Configure interjection probabilities with the `INTERJECTION_*_PROBABILITY` fields
11. For database functionality, add MySQL credentials
12. To enable/disable Google search, set `GOOGLE_SEARCH_ENABLED` to "true" or "false" (defaults to "true")
13. For AI responses, add Gemini API key

## Available Commands

- `!help` - Show help
- `!hello` - Say hello
- `!buzz` - Generate corporate buzzwords
- `!fightcrime` - Generate a crime fighting duo
- `!trump` - Generate a Trump insult
- `!bandname [name]` - Generate music genre for a band
- `!lastseen [name]` - Find when a user was last active
- `!quote [term]` - Get a random quote
- `!quote -show [show]` - Get quote from specific show
- `!quote -dud [user]` - Get random message from a user (or random user if no username provided)
- `!slogan [term]` - Get a random advertising slogan
- `!frinkiac [term]` - Get a Simpsons screenshot
- `!morbotron [term]` - Get a Futurama screenshot
- `!masterofallscience [term]` - Get a Rick and Morty screenshot
- `!imagine [text]` - Generate an image (if configured)
- `!alive [name]` - Check if a celebrity is alive or dead
- `!info` - Show bot statistics

## Database Structure

### Message History Database

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
5. Track edited messages to maintain accurate conversation context

The bot automatically manages its message history:
1. New messages are stored as they arrive with all metadata
2. Edited messages are updated to maintain accurate conversation context
3. The database is periodically trimmed to keep only the most recent messages (up to `MESSAGE_HISTORY_LIMIT`)
4. The trim interval can be configured with `DB_TRIM_INTERVAL_SECS` (defaults to 3600 seconds / 1 hour)
5. Existing databases are automatically migrated to the enhanced schema

### Quote Database Tables

The quote system uses MySQL and requires three related tables:

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

The slogan system uses a single MySQL table:

```sql
CREATE TABLE nuke_quotes (
    pn_id INT PRIMARY KEY AUTO_INCREMENT,
    pn_quote TEXT NOT NULL
);
```

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
1. Show a typing indicator while waiting for the API response
2. Send the content to Google's Gemini API with conversation context
3. Apply a realistic typing delay based on response length (0.2 seconds per word, minimum 2s, maximum 5s)
4. Post the AI-generated response as a reply to the user's message

When directly addressed (via mention or when a message starts with the bot's name), the bot will reply to the message, making it clear which message it's responding to. For other triggers like keyword detection, the bot will respond with a regular message.

The prompt sent to Gemini can be customized by setting the `GEMINI_PROMPT_WRAPPER` in your `CrowConfig.toml` file. The wrapper should include placeholders:
- `{message}` - The user's message
- `{bot_name}` - The bot's name
- `{user}` - The user's display name
- `{context}` - Recent conversation history (last 5 messages)

You can also configure which Gemini model to use by setting the `GEMINI_API_ENDPOINT` in your `CrowConfig.toml` file. This allows you to switch between different models like `gemini-1.0-pro`, `gemini-1.5-pro`, `gemini-1.5-flash` or `gemini-2.0-flash`.

## Conversation Context

The bot includes conversation context when making API calls to Gemini. This means:

1. The bot retrieves the last 5 messages from the conversation history
2. These messages are included in the prompt sent to Gemini
3. Gemini can generate more contextually relevant responses based on the conversation flow
4. The bot appears more coherent and can maintain conversation threads

This feature makes the bot feel more natural in conversations and helps it remember what was previously discussed.

## Random Interjections

The bot occasionally makes random interjections in the conversation. There are four types of interjections, each with its own configurable probability:

1. **MST3K Quotes** - Random quotes from Mystery Science Theater 3000
2. **Channel Memory** - Quotes something someone previously said in the channel
3. **Message Pondering** - Thoughtful comments about the conversation
4. **AI Interjection** - AI-generated comments using the Gemini API

Each interjection type can be configured separately in the `CrowConfig.toml` file:

```toml
# Random Interjection Probabilities (chance per message)
# Each type has its own probability - set to 0 to disable
INTERJECTION_MST3K_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_MEMORY_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_PONDERING_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_AI_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
```

Setting any probability to 0 will disable that type of interjection completely.
