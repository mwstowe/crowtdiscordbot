# Changelog

## [Unreleased]

### Added
- Context-aware Gemini API calls that include the last 5 messages from the conversation
- New `get_recent_messages` function in `db_utils.rs` to retrieve message history
- Enhanced `GeminiClient` with `generate_response_with_context` method

### Changed
- Updated all Gemini API calls to use conversation context
- Modified default prompt template to include a `{context}` placeholder

### Fixed
- Improved conversation coherence by providing context to the AI

## [1.0.0] - 2023-05-10

### Added
- Initial release
- Discord bot with command, mention, and keyword triggers
- Google search functionality
- Gemini API integration
- Quote and slogan database support
- Message history tracking
