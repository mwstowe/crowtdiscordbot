# Add MST3K Quote Interjection Feature

This pull request adds support for MST3K quote interjections by implementing the necessary database functionality.

## Implementation Details

1. Added `is_configured()` method to the `DatabaseManager` to check if the database is properly configured
2. Updated the `mst3k_quotes.rs` module to properly handle fallback scenarios

## How to Test

1. Configure the MySQL database connection in `CrowConfig.toml`
2. Set `INTERJECTION_MST3K_PROBABILITY` to a non-zero value (e.g., "0.005")
3. Run the bot and wait for a spontaneous MST3K quote interjection
4. Check the logs for "MST3K quote sent" messages

## Notes

- The MST3K quote interjection will only work if the MySQL database is properly configured
- The implementation respects the existing code structure and error handling patterns
- The implementation uses the existing database connection pool
