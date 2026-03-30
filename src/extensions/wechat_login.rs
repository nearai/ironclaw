use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Deserialize;
use uuid::Uuid;

use crate::extensions::{
    ExtensionError, InteractiveLoginInfo, InteractiveLoginPollResult, InteractiveLoginStartResult,
};

pub(crate) const WECHAT_CHANNEL_NAME: &str = "wechat";
pub(crate) const WECHAT_BASE_URL_SETTING_PATH: &str = "extensions.wechat.base_url";
pub(crate) const WECHAT_DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub(crate) const WECHAT_DEFAULT_BOT_TYPE: &str = "3";

const LOGIN_SESSION_TTL: Duration = Duration::from_secs(5 * 60);
const QR_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const QR_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_QR_REFRESH_COUNT: u8 = 3;
const WECHAT_ALLOWED_LOGIN_BASE_DOMAINS: &[&str] = &["weixin.qq.com", "wechat.com"];

#[derive(Debug, Clone)]
pub(crate) struct PendingWechatLogin {
    pub user_id: String,
    pub session_id: String,
    pub qrcode: String,
    pub qr_code_url: String,
    pub started_at: Instant,
    pub base_url: String,
    pub bot_type: String,
    pub refresh_count: u8,
}

impl PendingWechatLogin {
    pub fn is_fresh(&self) -> bool {
        self.started_at.elapsed() < LOGIN_SESSION_TTL
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConfirmedWechatLogin {
    pub bot_token: String,
    pub base_url: Option<String>,
    pub ilink_bot_id: String,
}

pub(crate) enum WechatLoginPollOutcome {
    Pending(InteractiveLoginPollResult),
    Confirmed(ConfirmedWechatLogin),
}

#[derive(Debug, Clone, Deserialize)]
struct QrCodeResponse {
    qrcode: String,
    qrcode_img_content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct QrStatusResponse {
    status: String,
    bot_token: Option<String>,
    ilink_bot_id: Option<String>,
    baseurl: Option<String>,
}

fn is_allowed_wechat_login_host(host: &str) -> bool {
    WECHAT_ALLOWED_LOGIN_BASE_DOMAINS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn validate_wechat_login_base_url(raw: &str) -> Result<String, ExtensionError> {
    // Trust model: WeChat QR login trusts the system CA store for HTTPS validation
    // to allowed WeChat domains. We do not certificate-pin iLink endpoints.
    let parsed = reqwest::Url::parse(raw).map_err(|e| {
        ExtensionError::AuthFailed(format!("WeChat login returned an invalid base URL: {e}"))
    })?;

    if parsed.scheme() != "https" {
        return Err(ExtensionError::AuthFailed(
            "WeChat login returned a non-HTTPS base URL".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ExtensionError::AuthFailed(
            "WeChat login returned a base URL with embedded credentials".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ExtensionError::AuthFailed(
            "WeChat login returned a base URL with an unexpected query or fragment".to_string(),
        ));
    }
    if parsed.path() != "/" && !parsed.path().is_empty() {
        return Err(ExtensionError::AuthFailed(
            "WeChat login returned a base URL with an unexpected path".to_string(),
        ));
    }

    let Some(host) = parsed.host_str() else {
        return Err(ExtensionError::AuthFailed(
            "WeChat login returned a base URL without a host".to_string(),
        ));
    };
    if !is_allowed_wechat_login_host(host) {
        return Err(ExtensionError::AuthFailed(format!(
            "WeChat login returned an untrusted base URL host: {host}"
        )));
    }

    Ok(format!("https://{host}"))
}

pub(crate) fn interactive_login_info() -> InteractiveLoginInfo {
    InteractiveLoginInfo {
        method: "qr_code".to_string(),
        button_label: "Connect WeChat".to_string(),
        instructions: Some("Scan the QR code with WeChat to connect this channel.".to_string()),
    }
}

pub(crate) fn purge_expired_logins(
    sessions: &mut std::collections::HashMap<String, PendingWechatLogin>,
) {
    sessions.retain(|_, session| session.is_fresh());
}

pub(crate) async fn start_login(
    user_id: &str,
    base_url: &str,
    bot_type: &str,
) -> Result<(PendingWechatLogin, InteractiveLoginStartResult), ExtensionError> {
    let qr = fetch_qr_code(base_url, bot_type).await?;
    Ok(build_pending_login(user_id, base_url, bot_type, qr))
}

pub(crate) async fn poll_login(
    session: &mut PendingWechatLogin,
) -> Result<WechatLoginPollOutcome, ExtensionError> {
    if !session.is_fresh() {
        return Ok(WechatLoginPollOutcome::Pending(
            InteractiveLoginPollResult {
                session_id: session.session_id.clone(),
                status: "failed".to_string(),
                message: "The QR code expired. Start a new WeChat connection.".to_string(),
                qr_code_url: None,
                activated: Some(false),
            },
        ));
    }

    let status = poll_qr_status(&session.base_url, &session.qrcode).await?;
    let refreshed_qr = if status.status == "expired" && session.refresh_count < MAX_QR_REFRESH_COUNT
    {
        Some(fetch_qr_code(&session.base_url, &session.bot_type).await?)
    } else {
        None
    };

    handle_poll_status(session, status, refreshed_qr)
}

fn build_pending_login(
    user_id: &str,
    base_url: &str,
    bot_type: &str,
    qr: QrCodeResponse,
) -> (PendingWechatLogin, InteractiveLoginStartResult) {
    let session_id = Uuid::new_v4().to_string();
    let session = PendingWechatLogin {
        user_id: user_id.to_string(),
        session_id: session_id.clone(),
        qrcode: qr.qrcode,
        qr_code_url: qr.qrcode_img_content.clone(),
        started_at: Instant::now(),
        base_url: base_url.to_string(),
        bot_type: bot_type.to_string(),
        refresh_count: 0,
    };

    let result = InteractiveLoginStartResult {
        session_id,
        status: "pending".to_string(),
        message: "Open the WeChat QR page to continue.".to_string(),
        qr_code_url: Some(qr.qrcode_img_content),
        instructions: Some(
            "Keep this window open while you scan and confirm on your phone.".to_string(),
        ),
    };

    (session, result)
}

fn handle_poll_status(
    session: &mut PendingWechatLogin,
    status: QrStatusResponse,
    refreshed_qr: Option<QrCodeResponse>,
) -> Result<WechatLoginPollOutcome, ExtensionError> {
    match status.status.as_str() {
        "wait" => Ok(WechatLoginPollOutcome::Pending(
            InteractiveLoginPollResult {
                session_id: session.session_id.clone(),
                status: "pending".to_string(),
                message: "Waiting for the QR code to be scanned.".to_string(),
                qr_code_url: None,
                activated: None,
            },
        )),
        "scaned" => Ok(WechatLoginPollOutcome::Pending(
            InteractiveLoginPollResult {
                session_id: session.session_id.clone(),
                status: "scanned".to_string(),
                message: "QR code scanned. Confirm the login in WeChat.".to_string(),
                qr_code_url: None,
                activated: None,
            },
        )),
        "expired" => {
            session.refresh_count = session.refresh_count.saturating_add(1);
            if session.refresh_count > MAX_QR_REFRESH_COUNT {
                return Ok(WechatLoginPollOutcome::Pending(
                    InteractiveLoginPollResult {
                        session_id: session.session_id.clone(),
                        status: "failed".to_string(),
                        message: "The QR code expired too many times. Start again.".to_string(),
                        qr_code_url: None,
                        activated: Some(false),
                    },
                ));
            }

            let refreshed = refreshed_qr.ok_or_else(|| {
                ExtensionError::Other(
                    "WeChat QR status expired without a refreshed QR code".to_string(),
                )
            })?;
            session.qrcode = refreshed.qrcode;
            session.qr_code_url = refreshed.qrcode_img_content.clone();
            session.started_at = Instant::now();

            Ok(WechatLoginPollOutcome::Pending(
                InteractiveLoginPollResult {
                    session_id: session.session_id.clone(),
                    status: "refreshed".to_string(),
                    message: "The QR code expired, so a fresh one was generated.".to_string(),
                    qr_code_url: Some(refreshed.qrcode_img_content),
                    activated: None,
                },
            ))
        }
        "confirmed" => {
            let bot_token = status.bot_token.filter(|token| !token.trim().is_empty());
            let ilink_bot_id = status
                .ilink_bot_id
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ExtensionError::Other(
                        "WeChat login succeeded but no bot account id was returned".to_string(),
                    )
                })?;

            let bot_token = bot_token.ok_or_else(|| {
                ExtensionError::Other(
                    "WeChat login succeeded but no bot token was returned".to_string(),
                )
            })?;
            let base_url = status
                .baseurl
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(validate_wechat_login_base_url)
                .transpose()?;

            Ok(WechatLoginPollOutcome::Confirmed(ConfirmedWechatLogin {
                bot_token,
                base_url,
                ilink_bot_id,
            }))
        }
        other => {
            tracing::warn!(status = other, "Unexpected WeChat QR status");
            Ok(WechatLoginPollOutcome::Pending(
                InteractiveLoginPollResult {
                    session_id: session.session_id.clone(),
                    status: "failed".to_string(),
                    message: format!("Unexpected WeChat login status: {other}"),
                    qr_code_url: None,
                    activated: Some(false),
                },
            ))
        }
    }
}

fn ensure_trailing_slash(base_url: &str) -> String {
    if base_url.ends_with('/') {
        base_url.to_string()
    } else {
        format!("{base_url}/")
    }
}

async fn fetch_qr_code(base_url: &str, bot_type: &str) -> Result<QrCodeResponse, ExtensionError> {
    let base = ensure_trailing_slash(base_url);
    let url = format!(
        "{base}ilink/bot/get_bot_qrcode?bot_type={}",
        urlencoding::encode(bot_type)
    );
    let client = Client::builder()
        .timeout(QR_FETCH_TIMEOUT)
        .build()
        .map_err(|e| ExtensionError::Other(format!("Failed to create WeChat login client: {e}")))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ExtensionError::Other(format!("Failed to fetch WeChat QR code: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(status = %status, "WeChat QR code request failed");
        return Err(ExtensionError::Other(format!(
            "WeChat QR code request failed with {status}: {body}"
        )));
    }

    response
        .json::<QrCodeResponse>()
        .await
        .map_err(|e| ExtensionError::Other(format!("Failed to parse WeChat QR code response: {e}")))
}

async fn poll_qr_status(base_url: &str, qrcode: &str) -> Result<QrStatusResponse, ExtensionError> {
    let base = ensure_trailing_slash(base_url);
    let url = format!(
        "{base}ilink/bot/get_qrcode_status?qrcode={}",
        urlencoding::encode(qrcode)
    );
    let client = Client::builder()
        .timeout(QR_LONG_POLL_TIMEOUT)
        .build()
        .map_err(|e| ExtensionError::Other(format!("Failed to create WeChat poll client: {e}")))?;

    let response = client
        .get(&url)
        .header("iLink-App-ClientVersion", "1")
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return Ok(QrStatusResponse {
                status: "wait".to_string(),
                bot_token: None,
                ilink_bot_id: None,
                baseurl: None,
            });
        }
        Err(error) => {
            return Err(ExtensionError::Other(format!(
                "Failed to poll WeChat QR status: {error}"
            )));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(status = %status, "WeChat QR status poll failed");
        return Err(ExtensionError::Other(format!(
            "WeChat QR status poll failed with {status}: {body}"
        )));
    }

    response
        .json::<QrStatusResponse>()
        .await
        .map_err(|e| ExtensionError::Other(format!("Failed to parse WeChat QR status: {e}")))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_QR_REFRESH_COUNT, QrCodeResponse, QrStatusResponse, WechatLoginPollOutcome,
        build_pending_login, handle_poll_status,
    };
    use crate::extensions::ExtensionError;

    #[test]
    fn test_build_pending_login_returns_qr_state_and_result() {
        let (session, start_result) = build_pending_login(
            "owner",
            "https://ilink.example",
            "3",
            QrCodeResponse {
                qrcode: "qr-123".to_string(),
                qrcode_img_content: "https://qr.example/one".to_string(),
            },
        );

        assert_eq!(session.user_id, "owner");
        assert_eq!(session.base_url, "https://ilink.example");
        assert_eq!(session.bot_type, "3");
        assert_eq!(session.qrcode, "qr-123");
        assert_eq!(session.qr_code_url, "https://qr.example/one");
        assert_eq!(start_result.status, "pending");
        assert_eq!(
            start_result.qr_code_url.as_deref(),
            Some("https://qr.example/one")
        );
        assert_eq!(start_result.session_id, session.session_id);
    }

    #[test]
    fn test_handle_poll_status_confirms_login() -> Result<(), String> {
        let (mut session, _) = build_pending_login(
            "owner",
            "https://ilink.example",
            "3",
            QrCodeResponse {
                qrcode: "qr-123".to_string(),
                qrcode_img_content: "https://qr.example/one".to_string(),
            },
        );
        let outcome = handle_poll_status(
            &mut session,
            QrStatusResponse {
                status: "confirmed".to_string(),
                bot_token: Some("bot-token-123".to_string()),
                ilink_bot_id: Some("wx-bot-1".to_string()),
                baseurl: Some("https://ilinkai.weixin.qq.com".to_string()),
            },
            None,
        )
        .map_err(|e| e.to_string())?;

        match outcome {
            WechatLoginPollOutcome::Confirmed(confirmed) => {
                assert_eq!(confirmed.bot_token, "bot-token-123");
                assert_eq!(confirmed.ilink_bot_id, "wx-bot-1");
                assert_eq!(
                    confirmed.base_url.as_deref(),
                    Some("https://ilinkai.weixin.qq.com")
                );
                Ok(())
            }
            WechatLoginPollOutcome::Pending(result) => Err(format!(
                "expected confirmed login, got pending status {}",
                result.status
            )),
        }
    }

    #[test]
    fn test_handle_poll_status_refreshes_expired_qr() -> Result<(), String> {
        let (mut session, _) = build_pending_login(
            "owner",
            "https://ilink.example",
            "3",
            QrCodeResponse {
                qrcode: "qr-initial".to_string(),
                qrcode_img_content: "https://qr.example/initial".to_string(),
            },
        );
        let outcome = handle_poll_status(
            &mut session,
            QrStatusResponse {
                status: "expired".to_string(),
                bot_token: None,
                ilink_bot_id: None,
                baseurl: None,
            },
            Some(QrCodeResponse {
                qrcode: "qr-refreshed".to_string(),
                qrcode_img_content: "https://qr.example/refreshed".to_string(),
            }),
        )
        .map_err(|e| e.to_string())?;

        match outcome {
            WechatLoginPollOutcome::Pending(result) => {
                assert_eq!(result.status, "refreshed");
                assert_eq!(
                    result.qr_code_url.as_deref(),
                    Some("https://qr.example/refreshed")
                );
                assert_eq!(session.qrcode, "qr-refreshed");
                assert_eq!(session.refresh_count, 1);
                Ok(())
            }
            WechatLoginPollOutcome::Confirmed(_) => {
                Err("expected QR refresh before confirmation".to_string())
            }
        }
    }

    #[test]
    fn test_handle_poll_status_fails_after_qr_refresh_exhaustion() -> Result<(), String> {
        let (mut session, _) = build_pending_login(
            "owner",
            "https://ilink.example",
            "3",
            QrCodeResponse {
                qrcode: "qr-initial".to_string(),
                qrcode_img_content: "https://qr.example/initial".to_string(),
            },
        );
        session.refresh_count = MAX_QR_REFRESH_COUNT;

        let outcome = handle_poll_status(
            &mut session,
            QrStatusResponse {
                status: "expired".to_string(),
                bot_token: None,
                ilink_bot_id: None,
                baseurl: None,
            },
            None,
        )
        .map_err(|e| e.to_string())?;

        match outcome {
            WechatLoginPollOutcome::Pending(result) => {
                assert_eq!(result.status, "failed");
                assert_eq!(result.activated, Some(false));
                assert!(
                    result.message.contains("expired too many times"),
                    "unexpected message: {}",
                    result.message
                );
                Ok(())
            }
            WechatLoginPollOutcome::Confirmed(_) => {
                Err("expected refresh exhaustion failure".to_string())
            }
        }
    }

    #[test]
    fn test_handle_poll_status_rejects_untrusted_base_url() {
        let (mut session, _) = build_pending_login(
            "owner",
            "https://ilink.example",
            "3",
            QrCodeResponse {
                qrcode: "qr-123".to_string(),
                qrcode_img_content: "https://qr.example/one".to_string(),
            },
        );

        let error = match handle_poll_status(
            &mut session,
            QrStatusResponse {
                status: "confirmed".to_string(),
                bot_token: Some("bot-token-123".to_string()),
                ilink_bot_id: Some("wx-bot-1".to_string()),
                baseurl: Some("https://evil.example".to_string()),
            },
            None,
        ) {
            Ok(_) => panic!("untrusted host should fail"),
            Err(error) => error,
        };

        match error {
            ExtensionError::AuthFailed(message) => {
                assert!(message.contains("untrusted base URL host"));
            }
            other => panic!("expected AuthFailed, got {other:?}"),
        }
    }
}
