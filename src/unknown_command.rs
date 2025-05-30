use anyhow::Result;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::error;
use crate::gemini_api::GeminiClient;

pub async fn handle_unknown_command(
    http: &Http, 
    msg: &Message, 
    command: &str,
    gemini_client: &GeminiClient,
    _ctx: &serenity::client::Context,  // Renamed to _ctx since we're not using it anymore
) -> Result<()> {
    // Show typing indicator while generating response
    if let Err(e) = msg.channel_id.broadcast_typing(http).await {
        error!("Failed to send typing indicator: {:?}", e);
    }
    
    // Create prompt for Gemini API
    let prompt = format!(
        "Create a humorous description of what a Discord bot command '!{}' would do, \
        followed by a funny reason why it was disabled. Format as: \
        '!{}: [description of what the command would do]\\n\\nDisabled because [funny reason]'. \
        Keep it concise (2-3 sentences max) and make it genuinely funny. \
        Examples: \
        '!time: Travel back in time to the specified period or a random period in history.\\n\\nDisabled because it keeps going back before it was implemented' \
        '!auto: Select and purchase an automobile on behalf of the user.\\n\\nDisabled after some poor sod received a cybertruck'",
        command, command
    );
    
    match gemini_client.generate_response(&prompt, "").await {
        Ok(response) => {
            // Send the response immediately without typing delay
            if let Err(e) = msg.channel_id.say(http, response).await {
                error!("Error sending unknown command response: {:?}", e);
            }
        },
        Err(e) => {
            error!("Error generating unknown command response: {:?}", e);
            // Don't send an error message to avoid confusion for unknown commands
        }
    }
    
    Ok(())
}
