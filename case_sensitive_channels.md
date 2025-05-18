# Case-Sensitive Channel Names Implementation

## Overview
The Crow Discord Bot now uses case-sensitive channel name matching to align with Discord's behavior. This ensures that users can specify the exact channel name they want to follow.

## Changes Made
In the `find_channels_by_name` function, the channel name comparison has been changed from case-insensitive to exact matching:

```rust
// Before:
if channel.name.to_lowercase() == name.to_lowercase() {
    info!("✅ Found matching channel '{}' (ID: {}) in server", channel.name, channel.id);
    found_channels.push(channel.id);
}

// After:
if channel.name == name {
    info!("✅ Found matching channel '{}' (ID: {}) in server", channel.name, channel.id);
    found_channels.push(channel.id);
}
```

## Impact
This change means that:

1. Channel names in the configuration file must exactly match the case used in Discord
2. If a channel is named "#General-Chat", specifying "general-chat" in the config will no longer match
3. This aligns with Discord's own behavior, which treats channel names as case-sensitive

## Testing
To test this functionality:
1. Create two channels with names that differ only in case (e.g., "test-channel" and "Test-Channel")
2. Configure the bot to follow one of them using the exact case
3. Verify that the bot only follows the specified channel and not the other

## Configuration Example
```toml
# Case-sensitive channel names
FOLLOWED_CHANNEL_NAMES = "general,announcements,Test-Channel"
```

In this example, the bot will only follow a channel named exactly "Test-Channel", not "test-channel" or "TEST-CHANNEL".
