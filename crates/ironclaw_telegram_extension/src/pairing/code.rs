use super::{
    PAIRING_CODE_ALPHABET, PAIRING_CODE_LEN, PairingCode, PairingIssue, TelegramPairingRecord,
};

pub(super) fn pairing_issue(record: &TelegramPairingRecord, bot_username: &str) -> PairingIssue {
    PairingIssue {
        code: record.code.clone(),
        deep_link: format!("https://t.me/{bot_username}?start={}", record.code),
        expires_at: record.expires_at,
    }
}

pub(super) fn mint_pairing_code() -> PairingCode {
    let code: String = (0..PAIRING_CODE_LEN)
        .map(|_| {
            let index = rand::random_range(0..PAIRING_CODE_ALPHABET.len());
            PAIRING_CODE_ALPHABET[index] as char
        })
        .collect();
    PairingCode::generated(code)
}
