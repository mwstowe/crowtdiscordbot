# Test Files

This directory contains test files used during development to verify functionality of various components of the Crow Discord Bot.

## Test Files

- `test_mst3k_quote.rs` - Tests the MST3K quote extraction from formatted quotes with speaker tags
- `test_cause_of_death.rs` - Tests the extraction of cause of death from Wikipedia text
- `test_celebrity_age.rs` - Tests the age calculation for celebrities
- `test_celebrity.rs` - Tests the celebrity status lookup functionality
- `test_extraction.rs` - Tests the extraction of dates from Wikipedia text
- `test_full_extraction.rs` - Tests the full extraction pipeline for celebrity information
- `fix_celebrity.rs` - Utility to fix celebrity data

## Running Tests

To run a specific test:

```bash
cargo run --bin tests/test_name
```

These tests are primarily for development and debugging purposes and are not part of the automated test suite.
