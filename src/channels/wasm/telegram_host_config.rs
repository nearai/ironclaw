pub const TELEGRAM_CHANNEL_NAME: &str = "telegram";
const TELEGRAM_BOT_USERNAME_SETTING_PREFIX: &str = "channels.wasm_channel_bot_usernames";

pub fn bot_username_setting_key(channel_name: &str) -> String {
    format!("{TELEGRAM_BOT_USERNAME_SETTING_PREFIX}.{channel_name}")
}
