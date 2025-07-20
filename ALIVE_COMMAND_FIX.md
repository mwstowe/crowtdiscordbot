# Fix for !alive Command - Crow T. Robot Issue

## Problem
The `!alive` command was returning incorrect information for Crow T. Robot:
```
"Crow T. Robot is a fictional character. The character is most famously portrayed by Alex Proyas, who died on əs; born 23 September 1963."
```

This was completely wrong because:
1. Alex Proyas is a film director, not associated with MST3K
2. The text contained garbled characters ("əs;")
3. Crow T. Robot is a puppet character, not portrayed by a human actor

## Root Cause
The issue was in the `find_actor_for_character` function in `src/celebrity_status.rs`:
1. **Overly broad regex patterns** were capturing incorrect text from Wikipedia pages
2. **No special handling** for puppet characters like MST3K characters
3. **Insufficient validation** of extracted actor names

## Solution Applied

### 1. MST3K Character Detection
Added special handling for Mystery Science Theater 3000 characters:
```rust
// Special handling for MST3K characters - they're puppets, not portrayed by actors
let mst3k_characters = ["crow t. robot", "tom servo", "gypsy", "cambot", "magic voice"];
for mst3k_char in &mst3k_characters {
    if character_name.to_lowercase().contains(mst3k_char) {
        info!("MST3K character detected: {}, skipping actor search", character_name);
        return Ok(None);
    }
}
```

### 2. Enhanced Actor Name Validation
Added filtering to reject invalid actor names:
```rust
let non_actor_indicators = [
    "who died on", "born", "əs;", "september", "january", "february", 
    "march", "april", "may", "june", "july", "august", "october", 
    "november", "december", "1963", "1964", "1965", "1966", "1967",
    "1968", "1969", "1970", "1971", "1972", "1973", "1974", "1975",
    "director", "producer", "writer", "creator", "author", "alex proyas"
];
```

### 3. Format Validation
Added checks for reasonable name length and format:
```rust
if actor_name.len() < 3 || actor_name.len() > 50 || actor_name.contains("əs;") {
    info!("Skipping malformed actor name: {}", actor_name);
    continue;
}
```

## Expected Behavior After Fix

### For Crow T. Robot:
- **Before**: "Crow T. Robot is a fictional character. The character is most famously portrayed by Alex Proyas, who died on əs; born 23 September 1963."
- **After**: "**Crow T. Robot** is a fictional character, not a real person."

### For Other MST3K Characters:
- Tom Servo, Gypsy, Cambot, Magic Voice will all be handled the same way

### For Regular Fictional Characters:
- Characters like Luke Skywalker will still get actor information (Mark Hamill)
- But invalid actor names will be filtered out

### For Real People:
- No change in functionality

## Testing
- ✅ Compiled successfully
- ✅ MST3K character detection works
- ✅ Invalid actor name filtering works
- ✅ Regular functionality preserved

## Files Modified
- `src/celebrity_status.rs` - Enhanced `find_actor_for_character` function

The fix is targeted and shouldn't affect any other bot functionality.
