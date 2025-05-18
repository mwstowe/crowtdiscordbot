# Separate Interjection Probabilities Implementation

## Overview
The Crow Discord Bot now has separate probability controls for each interjection type, replacing the single random interjection system.

## Changes Made

1. Added new configuration options for each interjection type:
```toml
# Random Interjection Probabilities (chance per message)
# Each type has its own probability - set to 0 to disable
INTERJECTION_MST3K_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_MEMORY_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_PONDERING_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
INTERJECTION_AI_PROBABILITY = "0.005"  # Default: 0.5% chance (1 in 200)
```

2. Added fields to the `Bot` struct to store these probabilities:
```rust
struct Bot {
    // ...existing fields...
    interjection_mst3k_probability: f64,
    interjection_memory_probability: f64,
    interjection_pondering_probability: f64,
    interjection_ai_probability: f64,
}
```

3. Updated the `Bot::new()` function to accept these parameters:
```rust
fn new(
    // ...existing parameters...
    interjection_mst3k_probability: f64,
    interjection_memory_probability: f64,
    interjection_pondering_probability: f64,
    interjection_ai_probability: f64,
) -> Self {
    // ...
}
```

4. Modified the `parse_config()` function to read these values:
```rust
// Parse interjection probabilities
let interjection_mst3k_probability = config.interjection_mst3k_probability
    .as_ref()
    .and_then(|prob| prob.parse::<f64>().ok())
    .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
    
let interjection_memory_probability = config.interjection_memory_probability
    .as_ref()
    .and_then(|prob| prob.parse::<f64>().ok())
    .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
    
let interjection_pondering_probability = config.interjection_pondering_probability
    .as_ref()
    .and_then(|prob| prob.parse::<f64>().ok())
    .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
    
let interjection_ai_probability = config.interjection_ai_probability
    .as_ref()
    .and_then(|prob| prob.parse::<f64>().ok())
    .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
```

5. Completely rewrote the interjection logic to check each type independently:
```rust
// Check for each type of interjection separately with their own probabilities
let mut interjection_made = false;

// MST3K Quote interjection
if rand::thread_rng().gen_bool(self.interjection_mst3k_probability) {
    info!("Triggered MST3K Quote interjection ({}% chance)", self.interjection_mst3k_probability * 100.0);
    // ... MST3K quote logic ...
    interjection_made = true;
}

// Channel Memory interjection
if !interjection_made && rand::thread_rng().gen_bool(self.interjection_memory_probability) {
    info!("Triggered Channel Memory interjection ({}% chance)", self.interjection_memory_probability * 100.0);
    // ... Channel memory logic ...
    interjection_made = true;
}

// Message Pondering interjection
if !interjection_made && rand::thread_rng().gen_bool(self.interjection_pondering_probability) {
    info!("Triggered Message Pondering interjection ({}% chance)", self.interjection_pondering_probability * 100.0);
    // ... Message pondering logic ...
    interjection_made = true;
}

// AI Interjection
if !interjection_made && rand::thread_rng().gen_bool(self.interjection_ai_probability) {
    info!("Triggered AI Interjection ({}% chance)", self.interjection_ai_probability * 100.0);
    // ... AI interjection logic ...
    interjection_made = true;
}
```

## Impact

This change allows:
1. Administrators to enable or disable specific types of interjections
2. Fine-tuning of the probability for each interjection type
3. Setting a probability to 0 to completely disable a specific interjection type
4. More control over the bot's personality and behavior

## Configuration Example

```toml
# Random Interjection Probabilities (chance per message)
# Each type has its own probability - set to 0 to disable
INTERJECTION_MST3K_PROBABILITY = "0.01"   # 1% chance (1 in 100)
INTERJECTION_MEMORY_PROBABILITY = "0.005"  # 0.5% chance (1 in 200)
INTERJECTION_PONDERING_PROBABILITY = "0"    # Disabled
INTERJECTION_AI_PROBABILITY = "0.02"   # 2% chance (1 in 50)
```

With this configuration, the bot will:
- Have a 1% chance to make MST3K quote interjections
- Have a 0.5% chance to make channel memory interjections
- Never make message pondering interjections (disabled)
- Have a 2% chance to make AI interjections
