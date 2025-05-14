The changes to src/gemini_api.rs and src/db_utils.rs have been successfully committed. To complete the implementation, you'll need to manually update src/main.rs to use the new context-aware functions. Here's what you need to do:

1. Find all occurrences of 'gemini_client.generate_response' in src/main.rs
2. Replace them with 'gemini_client.generate_response_with_context'
3. Before each call, add code to retrieve the last 5 messages from the database
4. Pass the context messages to the generate_response_with_context function

The changes are already in the codebase, but you'll need to manually integrate them into src/main.rs to avoid syntax errors.
