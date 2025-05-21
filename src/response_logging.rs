use tracing::info;

/// Log a direct message sent by the bot
pub fn log_direct_message(response: &str) {
    info!("Sent direct message: {}", response);
}

/// Log a reply message sent by the bot
pub fn log_reply(response: &str) {
    info!("Sent reply: {}", response);
}

/// Log an AI interjection sent by the bot
pub fn log_ai_interjection(response: &str) {
    info!("Sent AI interjection: {}", response);
}

/// Log an MST3K interjection sent by the bot
pub fn log_mst3k_interjection(response: &str) {
    info!("Sent MST3K interjection: {}", response);
}

/// Log a memory interjection sent by the bot
pub fn log_memory_interjection(response: &str) {
    info!("Sent memory interjection: {}", response);
}

/// Log a pondering interjection sent by the bot
pub fn log_pondering_interjection(response: &str) {
    info!("Sent pondering interjection: {}", response);
}
