# Changelog

## [Unreleased]

### Added
- Context-aware Gemini API calls that include the last 5 messages from the conversation
- New `get_recent_messages` function in `db_utils.rs` to retrieve message history
- Enhanced `GeminiClient` with `generate_response_with_context` method
- Enhanced database schema to store all message fields for proper conversion
- Automatic database migration system for upgrading existing databases
- Improved `load_message_history` function to properly reconstruct Message objects
- Realistic typing delay for bot responses based on message length
- Typing indicators while waiting for API responses and during typing delays
- Reply functionality when directly addressed by users
- Support for tracking edited messages in the database

### Changed
- Updated all Gemini API calls to use conversation context
- Modified default prompt template to include a `{context}` placeholder
- Enhanced `save_message` function to store all relevant fields from Discord Message objects
- Updated README.md with enhanced database schema information
- Added delay to responses (0.2 seconds per word, minimum 2s, maximum 5s) to make the bot feel more human-like
- Added typing indicators to make the bot's responses feel more natural
- Bot now replies to messages when directly addressed, making conversations clearer
- Modified Google search to skip sponsored results
- Removed "thinking..." message in favor of Discord's typing indicator

### Fixed
- Improved conversation coherence by providing context to the AI
- Fixed message history loading from database
- Resolved borrowing issues in database functions
- Added support for tracking edited messages to ensure conversation context is accurate

## [1.0.0] - 2023-05-10

### Added
- Initial release
- Discord bot with command, mention, and keyword triggers
- Google search functionality
- Gemini API integration
- Quote and slogan database support
- Message history tracking
