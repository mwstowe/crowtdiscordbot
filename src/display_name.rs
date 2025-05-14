use serenity::model::channel::Message;
use serenity::prelude::*;

// Helper function to get the best display name for a user
pub async fn get_best_display_name(ctx: &Context, msg: &Message) -> String {
    // Prioritize server nickname over global name over username
    if let Some(guild_id) = msg.guild_id {
        // Get member data which includes the nickname
        if let Ok(member) = guild_id.member(&ctx.http, msg.author.id).await {
            // Use nickname if available, otherwise fall back to global name or username
            if let Some(nick) = member.nick {
                return nick;
            }
        }
    }
    
    // Fall back to global name or username if no nickname or couldn't get member data
    msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone())
}

// Clean a display name by removing IRC formatting and brackets
pub fn clean_display_name(name: &str) -> String {
    let mut clean_name = name.to_string();
    clean_name = clean_name.replace("<", "").replace(">", "");
    clean_name = clean_name.replace("[irc]", "").trim().to_string();
    clean_name
}
