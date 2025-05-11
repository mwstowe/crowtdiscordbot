# Changes to Implement in Crow Discord Bot

Here are the changes needed to make the bot recognize when it's being addressed by name at the beginning of a message and include the user's display name in Gemini API calls:

## 1. Update the prompt wrapper template

Change:
```rust
gemini_prompt_wrapper: "You are {bot_name}, a helpful and friendly Discord bot. Respond to: {message}".to_string(),
```

To:
```rust
gemini_prompt_wrapper: "You are {bot_name}, a helpful and friendly Discord bot. Respond to {user}: {message}".to_string(),
```

## 2. Add a new method to handle user-specific API calls

```rust
async fn call_gemini_api_with_user(&self, prompt: &str, user_name: &str) -> Result<String> {
    // Check if we have an API key
    let api_key = match &self.gemini_api_key {
        Some(key) => key,
        None => {
            return Err(anyhow::anyhow!("Gemini API key not configured"));
        }
    };
    
    // Determine which endpoint to use
    let endpoint = self.gemini_api_endpoint.as_deref().unwrap_or("https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent");
    
    // Format the prompt using the wrapper, including the user's name
    let formatted_prompt = self.gemini_prompt_wrapper
        .replace("{message}", prompt)
        .replace("{bot_name}", &self.bot_name)
        .replace("{user}", user_name);
    
    info!("Calling Gemini API with prompt: {}", formatted_prompt);
    
    // Create the request body
    let request_body = serde_json::json!({
        "contents": [{
            "parts": [{
                "text": formatted_prompt
            }]
        }],
        "generationConfig": {
            "temperature": 0.7,
            "topK": 40,
            "topP": 0.95,
            "maxOutputTokens": 1024
        }
    });
    
    // Create the client
    let client = reqwest::Client::new();
    
    // Make the request
    let response = client.post(format!("{}?key={}", endpoint, api_key))
        .header("Content-Type", "application/json")
        .body(request_body.to_string())
        .send()
        .await?;
    
    // Check if the request was successful
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Gemini API request failed: {}", error_text));
    }
    
    // Parse the response
    let response_json: serde_json::Value = response.json().await?;
    
    // Extract the generated text
    let generated_text = response_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Failed to extract text from Gemini API response"))?
        .to_string();
    
    Ok(generated_text)
}
```

## 3. Update the existing API call method to use the new one

```rust
async fn call_gemini_api(&self, prompt: &str) -> Result<String> {
    // For backward compatibility, call the version with user name
    self.call_gemini_api_with_user(prompt, "User").await
}
```

## 4. Update the direct mention handler to use the new method

Change:
```rust
// Call the Gemini API
match self.call_gemini_api(&content).await {
```

To:
```rust
// Call the Gemini API with user's display name
match self.call_gemini_api_with_user(&content, &msg.author.name).await {
```

## 5. Add code to detect when the bot's name is at the beginning of a message

Add this code before the "Check for keyword triggers" section:

```rust
// Check if message starts with the bot's name
let content_lower = msg.content.to_lowercase();
let bot_name_lower = self.bot_name.to_lowercase();

if content_lower.starts_with(&bot_name_lower) {
    // Extract the message content without the bot's name
    let content = msg.content[self.bot_name.len()..].trim().to_string();
    
    if !content.is_empty() {
        if let Some(_api_key) = &self.gemini_api_key {
            // Send a "thinking" message
            let thinking_msg = match msg.channel_id.say(&ctx.http, "*thinking...*").await {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Error sending thinking message: {:?}", e);
                    return Ok(());
                }
            };
            
            // Call the Gemini API with user's display name
            match self.call_gemini_api_with_user(&content, &msg.author.name).await {
                Ok(response) => {
                    // Edit the thinking message with the actual response
                    if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(response)).await {
                        error!("Error editing thinking message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Sorry, I couldn't edit my message. Here's my response:").await {
                            error!("Error sending fallback message: {:?}", e);
                        }
                        if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                            error!("Error sending Gemini response: {:?}", e);
                        }
                    }
                },
                Err(e) => {
                    error!("Error calling Gemini API: {:?}", e);
                    if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(format!("Sorry, I encountered an error: {}", e))).await {
                        error!("Error editing thinking message: {:?}", e);
                    }
                }
            }
        } else {
            // Fallback if Gemini API is not configured
            if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you called my name! I'm {}! (Gemini API is not configured)", msg.author.name, self.bot_name)).await {
                error!("Error sending name response: {:?}", e);
            }
        }
        return Ok(());
    }
}
```
