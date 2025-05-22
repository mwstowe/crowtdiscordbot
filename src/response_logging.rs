use tracing::{info, debug};

/// Log a direct message sent by the bot
pub fn log_direct_message(message: &str) {
    info!("📤 Direct message sent: {}", message);
}

/// Log a reply message sent by the bot
pub fn log_reply(message: &str) {
    info!("↩️ Reply sent: {}", message);
}

/// Log an AI-generated interjection
pub fn log_ai_interjection(message: &str) {
    info!("🤖 AI interjection: {}", message);
}

/// Log an MST3K quote interjection
pub fn log_mst3k_interjection(message: &str) {
    info!("🎬 MST3K interjection: {}", message);
}

/// Log a memory interjection (quoting previous messages)
pub fn log_memory_interjection(message: &str) {
    info!("💭 Memory interjection: {}", message);
}

/// Log a pondering interjection
pub fn log_pondering_interjection(message: &str) {
    info!("🤔 Pondering interjection: {}", message);
}
