# Response Logging Module for Crow Discord Bot

This document outlines the implementation of a new response logging module for the Crow Discord Bot.

## Overview

The response logging module provides standardized logging functions for different types of bot responses, making it easier to track and debug bot interactions. Each function adds appropriate emoji and context to the log messages.

## Implementation

The module is implemented in `src/response_logging.rs` and provides the following functions:

```rust
/// Log a direct message sent by the bot
pub fn log_direct_message(message: &str) {
    info!("📤 Direct message sent: {}", message);
}

/// Log a reply message sent by the bot
pub fn log_reply(message: &str) {
    info!("↩️ Reply sent: {}", message);
}

/// Log an AI-generated interjection
pub fn log_ai_interjection(message: &str) {
    info!("🤖 AI interjection: {}", message);
}

/// Log an MST3K quote interjection
pub fn log_mst3k_interjection(message: &str) {
    info!("🎬 MST3K interjection: {}", message);
}

/// Log a memory interjection (quoting previous messages)
pub fn log_memory_interjection(message: &str) {
    info!("💭 Memory interjection: {}", message);
}

/// Log a pondering interjection
pub fn log_pondering_interjection(message: &str) {
    info!("🤔 Pondering interjection: {}", message);
}
```

## Integration Points

To use these logging functions, the following changes need to be made to the main bot code:

1. Import the module at the top of `main.rs`:
   ```rust
   mod response_logging;
   use response_logging::{log_direct_message, log_reply, log_ai_interjection, 
                         log_mst3k_interjection, log_memory_interjection, 
                         log_pondering_interjection};
   ```

2. Replace existing logging calls with the appropriate specialized function:

   - For direct messages:
     ```rust
     log_direct_message(&message);
     ```

   - For reply messages:
     ```rust
     log_reply(&message);
     ```

   - For AI interjections:
     ```rust
     log_ai_interjection(&message);
     ```

   - For MST3K quote interjections:
     ```rust
     log_mst3k_interjection(&message);
     ```

   - For memory interjections:
     ```rust
     log_memory_interjection(&message);
     ```

   - For pondering interjections:
     ```rust
     log_pondering_interjection(&message);
     ```

## Benefits

1. **Consistent Formatting**: All log messages follow a consistent format with appropriate emoji
2. **Improved Readability**: Different types of messages are visually distinct in logs
3. **Easier Filtering**: The emoji prefixes make it easier to filter logs by message type
4. **Centralized Logging Logic**: All logging format changes can be made in one place
5. **Better Debugging**: More context about the type of message being sent

## Future Enhancements

1. Add log levels for different types of messages
2. Add more detailed context (e.g., channel name, server name)
3. Add optional message truncation for very long messages
4. Add statistics tracking for different message types
5. Implement log rotation or external logging service integration
