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

### Changed
- Updated all Gemini API calls to use conversation context
- Modified default prompt template to include a `{context}` placeholder
- Enhanced `save_message` function to store all relevant fields from Discord Message objects
- Updated README.md with enhanced database schema information
- Added delay to responses (0.5 seconds per word) to make the bot feel more human-like

### Fixed
- Improved conversation coherence by providing context to the AI
- Fixed message history loading from database
- Resolved borrowing issues in database functions

## [1.0.0] - 2023-05-10

### Added
- Initial release
- Discord bot with command, mention, and keyword triggers
- Google search functionality
- Gemini API integration
- Quote and slogan database support
- Message history tracking
