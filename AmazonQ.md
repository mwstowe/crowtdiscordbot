# Crow Discord Bot - Changes Made with Amazon Q

## Interjection Probability and Silence Period Adjustments

### Changes Made:
1. **Reduced Default Interjection Probabilities**
   - Decreased all interjection probabilities by half (from 0.005 to 0.0025)
   - This reduces the frequency of random interjections from 1 in 200 messages to 1 in 400 messages
   - Affects all interjection types: MST3K quotes, memory, pondering, AI, facts, and news

2. **Extended Silence Period**
   - Changed the silence period from 1 hour to 90 minutes (1.5 hours)
   - The bot now waits 90 minutes of channel inactivity before starting to increase interjection probabilities
   - This gives users more time between bot interjections during periods of low activity

### Files Modified:
- `CrowConfig.toml.example` - Updated default configuration values

### Technical Details:
- The `INTERJECTION_*_PROBABILITY` values were all reduced from 0.005 to 0.0025
- The `FILL_SILENCE_START_HOURS` value was increased from 1.0 to 1.5
- The `FILL_SILENCE_MAX_HOURS` value remains unchanged at 12 hours

These changes make the bot less intrusive during normal conversation while still maintaining the ability to keep channels active during extended periods of silence.
