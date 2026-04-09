use crate::gemini_api::GeminiClient;
use anyhow::Result;
use tracing::{error, info};

/// Verify that a news article title and summary match the content at the URL
pub async fn verify_news_article(
    gemini_client: &GeminiClient,
    article_title: &str,
    article_url: &str,
    article_summary: &str,
) -> Result<bool> {
    // First, fetch the actual page content
    info!("Fetching page content from: {}", article_url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?;

    let response = match client.get(article_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Failed to fetch URL {}: {:?}", article_url, e);
            return Ok(false);
        }
    };

    // Check status code - only accept 200
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        error!("URL returned non-200 status: {}", status);
        return Ok(false);
    }

    // Check if we were redirected to a different domain (often indicates removed content)
    let final_url = response.url().to_string();
    if let (Ok(original), Ok(final_parsed)) =
        (url::Url::parse(article_url), url::Url::parse(&final_url))
    {
        if original.domain() != final_parsed.domain() {
            error!(
                "URL redirected to different domain: {} -> {}",
                article_url, final_url
            );
            return Ok(false);
        }
    }

    // Get the page content
    let page_content = match response.text().await {
        Ok(content) => content,
        Err(e) => {
            error!("Failed to read page content: {:?}", e);
            return Ok(false);
        }
    };

    // Check for soft 404 indicators in the raw HTML
    let content_lower = page_content.to_lowercase();
    let soft_404_indicators = [
        "page not found",
        "404 error",
        "page doesn't exist",
        "page does not exist",
        "page you're looking for",
        "page you are looking for",
        "content not found",
        "this page doesn't exist",
        "this page does not exist",
        "page has been removed",
        "page has been deleted",
        "no longer available",
        "page cannot be found",
        "sorry, we couldn't find",
        "oops! that page can't be found",
    ];

    for indicator in &soft_404_indicators {
        if content_lower.contains(indicator) {
            error!("Soft 404 detected - page contains: '{}'", indicator);
            return Ok(false);
        }
    }

    // Extract text from HTML (simple approach - just get text between tags)
    let text_content = extract_text_from_html(&page_content);

    // Check if content is too short (likely an error page)
    if text_content.len() < 200 {
        error!(
            "Page content too short ({} chars) - likely an error page",
            text_content.len()
        );
        return Ok(false);
    }

    // Take first 2000 characters of content for verification
    let content_sample = if text_content.len() > 2000 {
        &text_content[..2000]
    } else {
        &text_content
    };

    // Create a prompt for Gemini to verify the article against actual content
    let prompt = format!(
        "You are verifying if an article title and summary match the actual page content.\n\n\
        Article Title: {article_title}\n\
        Article Summary: {article_summary}\n\n\
        Actual Page Content (first 2000 chars):\n{content_sample}\n\n\
        Does the title and summary accurately describe this page content?\n\
        Consider:\n\
        1. Is this actually a news article (not a 404, category page, or error page)?\n\
        2. Does the title match the main topic of the content?\n\
        3. Does the summary accurately reflect what's in the content?\n\
        4. Is the content substantive (not just a stub or redirect)?\n\n\
        Respond with ONLY ONE word:\n\
        - \"MATCH\" - if title/summary accurately match the content\n\
        - \"MISMATCH\" - if there's a clear mismatch or the page doesn't exist/is broken"
    );

    // Send the prompt to Gemini
    match gemini_client.generate_content(&prompt).await {
        Ok(response) => {
            let response = response.trim().to_uppercase();
            info!("News verification response: {}", response);

            match response.as_str() {
                "MATCH" => Ok(true),
                _ => {
                    error!("News verification failed: {}", response);
                    Ok(false)
                }
            }
        }
        Err(e) => {
            error!("Error verifying news article: {:?}", e);
            Ok(false)
        }
    }
}

/// Extract text content from HTML (simple tag stripping)
fn extract_text_from_html(html: &str) -> String {
    // Remove script and style tags and their content
    let mut text = html.to_string();

    // Remove script tags
    while let Some(start) = text.find("<script") {
        if let Some(end) = text[start..].find("</script>") {
            text.replace_range(start..start + end + 9, " ");
        } else {
            break;
        }
    }

    // Remove style tags
    while let Some(start) = text.find("<style") {
        if let Some(end) = text[start..].find("</style>") {
            text.replace_range(start..start + end + 8, " ");
        } else {
            break;
        }
    }

    // Remove all HTML tags
    let mut result = String::new();
    let mut in_tag = false;

    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Clean up whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the article title and URL from a formatted news interjection
pub fn extract_article_info(text: &str) -> Option<(String, String)> {
    // Look for the pattern "Article title: https://..."
    if let Some(colon_pos) = text.find(": http") {
        let title = text[0..colon_pos].trim().to_string();

        // Find the end of the URL (space, newline, or end of string)
        let url_start = colon_pos + 2; // Skip the ": "
        let url_end = text[url_start..]
            .find(|c: char| c.is_whitespace())
            .map_or(text.len(), |pos| url_start + pos);

        let url = text[url_start..url_end].trim().to_string();

        // Extract the summary (everything after the URL)
        let _summary = if url_end < text.len() {
            text[url_end..].trim().to_string()
        } else {
            String::new()
        };

        Some((title, url))
    } else {
        None
    }
}
