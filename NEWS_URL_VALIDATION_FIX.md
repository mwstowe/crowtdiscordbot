# Fix for News Interjection URL Validation

## Problem
The bot was generating news interjections with invalid URLs pointing to archive/category pages instead of specific articles. Example:

```
"Article Title: "AI Now Able to Detect Sarcasm with 60% Accuracy, Still >
Article URL: https://arstechnica.com/ai/2024/04/"
```

The URL `https://arstechnica.com/ai/2024/04/` points to an archive page (April 2024 AI articles) rather than a specific article, making the interjection misleading and unhelpful.

## Root Cause
The `verify_url_format` function in `src/news_verification.rs` was not effectively detecting and filtering out archive/category page URLs. It had basic validation but missed common patterns like:
- Category/year/month archives (`/ai/2024/04/`)
- Category/year archives (`/news/2024/`)
- Pure category pages (`/technology/`)
- Tag/archive pages (`/tag/ai/`, `/archive/`)

## Solution Applied

### Enhanced Archive/Category Detection
Added a new `is_archive_or_category_url` function that detects multiple patterns:

1. **Category/Year/Month Pattern**: `/ai/2024/04/` or `/science/2024/12/`
2. **Category/Year Pattern**: `/news/2024/` or `/tech/2024/`
3. **Pure Category Pages**: `/ai/`, `/technology/`, `/science/`
4. **Archive Indicators**: URLs containing `/archive/`, `/category/`, `/tag/`, `/page/`

### Improved URL Structure Validation
- Better parsing of URL segments (excluding protocol and domain)
- More robust checking for article title segments (minimum 3 words)
- Detection of date-only segments that indicate archive pages
- Validation that URLs have sufficient depth to be specific articles

### Helper Functions Added
- `is_year()`: Validates 4-digit years (1990-2030)
- `is_month()`: Validates 2-digit months (01-12)
- `is_date_segment()`: Detects YYYY-MM-DD format segments

## Test Results

### URLs Now Correctly REJECTED:
- ❌ `https://arstechnica.com/ai/2024/04/` (the problematic URL)
- ❌ `https://techcrunch.com/category/ai/2024/`
- ❌ `https://wired.com/technology/`
- ❌ `https://wired.com/tag/artificial-intelligence/`

### URLs Still Correctly ACCEPTED:
- ✅ `https://arstechnica.com/ai/2024/04/new-ai-breakthrough-changes-everything/`
- ✅ `https://techcrunch.com/2024/04/15/startup-raises-million-for-ai-platform/`
- ✅ `https://wired.com/story/artificial-intelligence-transforms-healthcare-industry/`

## Expected Behavior After Fix

### Before:
Bot could generate interjections with archive URLs like:
```
"Article Title: AI Now Able to Detect Sarcasm with 60% Accuracy, Still >
Article URL: https://arstechnica.com/ai/2024/04/"
```

### After:
- Archive/category URLs will be rejected during validation
- Only specific article URLs will pass validation
- News interjections will be more reliable and useful
- Users won't be directed to generic archive pages

## Files Modified
- `src/news_verification.rs` - Enhanced `verify_url_format()` function and added helper functions

## Impact
- Improved quality of news interjections
- Reduced user frustration from broken/misleading links
- Better adherence to the bot's purpose of sharing interesting specific articles
- No impact on other bot functionality

The fix ensures that news interjections only include links to actual articles, not archive or category pages.
