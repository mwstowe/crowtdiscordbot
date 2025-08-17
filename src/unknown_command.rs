use crate::gemini_api::GeminiClient;
use crate::is_prompt_echo;
use anyhow::Result;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::error;

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
    let command_with_exclamation = format!("!{command_name}");

    // Find where the command ends and parameters begin
    let params = if content.len() > command_with_exclamation.len() {
        content[command_with_exclamation.len()..].trim()
    } else {
        ""
    };

    // Create prompt for Gemini API based on whether there are parameters
    let prompt = if !params.is_empty() {
        format!(
            "You are analyzing an unknown Discord bot command '!{command_name}' with parameter '{params}'. \
            First, determine what category of parameter this is (e.g., [username], [time], [location], [item], etc.). \
            Then create a humorous description of what this command would do, followed by a funny reason why it was disabled. \
            Format your response EXACTLY as: \
            '!{command_name} [parameter_category]: [description of what the command would do with this type of parameter]\n\nDisabled because [funny reason]'. \
            Keep it concise (2-3 sentences max) and make it genuinely funny. \
            DO NOT include any introductory text, commentary, or explanations. \
            DO NOT include phrases like 'Here's my attempt' or 'I've got more'. \
            ONLY return the formatted command description. \
            Examples: \
            '!time [year]: Travel back in time to the specified year.\n\nDisabled because too many users were trying to meet dinosaurs' \
            '!weather [location]: Check the current weather conditions at the specified location.\n\nDisabled after the bot kept reporting \"cloudy with a chance of server crashes\"'"
        )
    } else {
        format!(
            "Create a humorous description of what a Discord bot command '!{command_name}' would do, \
            followed by a funny reason why it was disabled. Format EXACTLY as: \
            '!{command_name}: [description of what the command would do]\n\nDisabled because [funny reason]'. \
            Keep it concise (2-3 sentences max) and make it genuinely funny. \
            DO NOT include any introductory text, commentary, or explanations. \
            DO NOT include phrases like 'Here's my attempt' or 'I've got more'. \
            ONLY return the formatted command description. \
            Examples: \
            '!time: Travel back in time to the specified period or a random period in history.\n\nDisabled because it keeps going back before it was implemented' \
            '!auto: Select and purchase an automobile on behalf of the user.\n\nDisabled after some poor sod received a cybertruck'"
        )
    };

    match gemini_client
        .generate_response_with_context(&prompt, "", &Vec::new(), None)
        .await
    {
        Ok(response) => {
            // Check if the response looks like a prompt echo
            if is_prompt_echo(&response) {
                error!(
                    "Unknown command response error: API returned the prompt instead of a response"
                );

                // Send a generic error message
                if let Err(e) = msg
                    .channel_id
                    .say(http, "Sorry, I couldn't process that command right now.")
                    .await
                {
                    error!("Error sending error message: {:?}", e);
                }
                return Ok(());
            }

            // Replace literal "\n\n" with actual newlines
            let fixed_response = response.replace("\\n\\n", "\n\n");

            // Send the response immediately without typing delay
            if let Err(e) = msg.channel_id.say(http, fixed_response).await {
                error!("Error sending unknown command response: {:?}", e);
            }
        }
        Err(e) => {
            error!("Error generating unknown command response: {:?}", e);

            // Check if this is a silent error (overload)
            if e.to_string().contains("SILENT_ERROR") {
                error!("Gemini API overloaded, not sending error message to channel");
                return Ok(());
            }

            // For other errors, send a generic error message
            if let Err(e) = msg
                .channel_id
                .say(http, "Sorry, I couldn't process that command right now.")
                .await
            {
                error!("Error sending error message: {:?}", e);
            }
        }
    }

    Ok(())
}
