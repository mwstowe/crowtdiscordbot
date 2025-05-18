# Crow Discord Bot Improvements

## 1. Channel-Specific Message History

Added channel-specific message history to ensure the bot maintains separate conversation contexts for each channel. This prevents cross-channel confusion and makes the bot's responses more contextually relevant.

### Implementation:
- Modified `get_recent_messages()` to accept an optional `channel_id` parameter
- Added SQL filtering to retrieve messages only from the specified channel
- Updated all calls to include the current channel ID when retrieving context

## 2. Case-Sensitive Channel Names

Made channel names case-sensitive in the configuration file to match Discord's behavior. This ensures that users can specify the exact channel name they want to follow.

### Implementation:
- Changed the channel name comparison from case-insensitive to exact matching
- Maintained case-insensitivity for configuration variable names

## 3. Configurable Context Messages

Added the ability to configure the number of context messages sent to the Gemini API and display them in chronological order.

### Implementation:
- Added a new configuration option `gemini_context_messages` to control how many messages are included
- Modified the context retrieval to reverse the order of messages (oldest first)
- Updated the Gemini client to use the configured number of messages

### Configuration Example:
```toml
# Number of previous messages to include as context for AI responses
GEMINI_CONTEXT_MESSAGES = "5"
```

## 4. Separate Interjection Probabilities

Replaced the single random interjection system with separate probability controls for each interjection type.

### Implementation:
- Added configuration options for each interjection type:
  - `INTERJECTION_MST3K_PROBABILITY`
  - `INTERJECTION_MEMORY_PROBABILITY`
  - `INTERJECTION_PONDERING_PROBABILITY`
  - `INTERJECTION_AI_PROBABILITY`
- Modified the interjection logic to check each type independently
- Set default probabilities to 0.5% (1 in 200) for each type

### Configuration Example:
```toml
# Random Interjection Probabilities (chance per message)
# Each type has its own probability - set to 0 to disable
INTERJECTION_MST3K_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_MEMORY_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_PONDERING_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_AI_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
```

These improvements make the bot more configurable and provide better context-aware responses in multi-channel environments.
