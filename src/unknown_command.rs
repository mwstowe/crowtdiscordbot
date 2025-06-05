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
    _ctx: &serenity::client::Context,
) -> Result<()> {
    // Show typing indicator while generating response
    if let Err(e) = msg.channel_id.broadcast_typing(http).await {
        error!("Failed to send typing indicator: {:?}", e);
    }
    
    // Extract just the command part (without the !)
    let command_name = if command.starts_with('!') {
        &command[1..]
    } else {
        command
    };
    
    // Check if there are parameters after the command
    let content = msg.content.trim();
    let command_with_exclamation = format!("!{}", command_name);
    
    // Find where the command ends and parameters begin
    let params = if content.len() > command_with_exclamation.len() {
        content[command_with_exclamation.len()..].trim()
    } else {
        ""
    };
    
    // Create prompt for Gemini API based on whether there are parameters
    let prompt = if !params.is_empty() {
        format!(
            "Create a humorous description of what a Discord bot command '!{}' with parameter '{}' would do, \
            followed by a funny reason why it was disabled. Format as: \
            '!{} {}: [description of what the command would do with this specific parameter]\\n\\nDisabled because [funny reason]'. \
            Keep it concise (2-3 sentences max) and make it genuinely funny. \
            Examples: \
            '!time 1985: Travel back in time specifically to 1985.\\n\\nDisabled because too many users were trying to meet Marty McFly' \
            '!weather Mars: Check the current weather conditions on Mars.\\n\\nDisabled after the bot kept reporting \"dusty with a chance of rovers\"'",
            command_name, params, command_name, params
        )
    } else {
        format!(
            "Create a humorous description of what a Discord bot command '!{}' would do, \
            followed by a funny reason why it was disabled. Format as: \
            '!{}: [description of what the command would do]\\n\\nDisabled because [funny reason]'. \
            Keep it concise (2-3 sentences max) and make it genuinely funny. \
            Examples: \
            '!time: Travel back in time to the specified period or a random period in history.\\n\\nDisabled because it keeps going back before it was implemented' \
            '!auto: Select and purchase an automobile on behalf of the user.\\n\\nDisabled after some poor sod received a cybertruck'",
            command_name, command_name
        )
    };
    
    match gemini_client.generate_response_with_context(&prompt, "", &Vec::new(), None).await {
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
