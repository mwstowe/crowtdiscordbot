# Configurable Context Messages Implementation

## Overview
The Crow Discord Bot now allows configuring the number of context messages sent to the Gemini API and displays them in chronological order (oldest first).

## Changes Made

1. Added a new configuration option `gemini_context_messages` to control how many messages are included:

```toml
# Number of previous messages to include as context for AI responses
GEMINI_CONTEXT_MESSAGES = "5"
```

2. Added a field to the `Bot` struct to store the configured value:

```rust
struct Bot {
    // ...existing fields...
    gemini_context_messages: usize,
}
```

3. Updated the `Bot::new()` function to accept this parameter:

```rust
fn new(
    // ...existing parameters...
    gemini_context_messages: usize,
) -> Self {
    // ...
}
```

4. Modified the `parse_config()` function to read this value:

```rust
// Parse number of context messages to include in Gemini API calls
let gemini_context_messages = config.gemini_context_messages
    .as_ref()
    .and_then(|count| count.parse::<usize>().ok())
    .unwrap_or(5); // Default: 5 messages
    
info!("Gemini API context messages set to {}", gemini_context_messages);
```

5. Updated the `GeminiClient` constructor to accept this parameter:

```rust
Some(GeminiClient::new(
    api_key, 
    gemini_api_endpoint,
    gemini_prompt_wrapper,
    bot_name.clone(),
    gemini_rate_limit_minute,
    gemini_rate_limit_day,
    gemini_context_messages // Use configured context messages
))
```

6. Modified all calls to `get_recent_messages()` to use the configured value:

```rust
match db_utils::get_recent_messages(db.clone(), self.gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
    // ...
}
```

7. The `GeminiClient` already had logic to reverse the order of messages to get chronological order (oldest first):

```rust
// Reverse the messages to get chronological order (oldest first)
let mut chronological_messages = limited_messages.to_vec();
chronological_messages.reverse();
```

## Impact

This change allows:
1. Administrators to configure how much conversation history is included in AI prompts
2. Messages to be presented in chronological order (oldest first) for better context understanding
3. Fine-tuning of the context window based on the specific use case and available API tokens

## Configuration Example

```toml
# Number of previous messages to include as context for AI responses
GEMINI_CONTEXT_MESSAGES = "10"  # Include 10 previous messages for more context
```

With this configuration, the bot will include up to 10 previous messages as context when generating AI responses, instead of the default 5.
