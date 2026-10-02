//! Outgoing email: what ActionMailer and Email::Sender do in Rails.
//!
//! Discourse is an SMTP client; it relays through the server named by the
//! `DISCOURSE_SMTP_*` settings (GlobalSetting.smtp_settings), or hands mail
//! to sendmail when none is set. Messages are built here as headers plus a
//! text and an HTML part, then encoded and delivered with lettre. Tests and
//! recordings keep them in memory instead.

pub mod notification;
pub mod sender;
pub mod styles;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::config::GlobalSettings;
use crate::{AppError, Unsupported};

/// A message as Rails' mail object holds it: header fields in order, the
/// text part and the HTML part.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub headers: Vec<(String, String)>,
    pub text: String,
    pub html: Option<String>,
}

impl Message {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `message.header[name] = value`: replaces the field in place, or
    /// appends it; `None` removes it.
    pub fn set_header(&mut self, name: &str, value: Option<String>) {
        let position = self
            .headers
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(name));
        match (position, value) {
            (Some(i), Some(v)) => self.headers[i].1 = v,
            (Some(i), None) => {
                self.headers.remove(i);
            }
            (None, Some(v)) => self.headers.push((name.to_string(), v)),
            (None, None) => {}
        }
    }
}

/// Where messages go.
#[derive(Clone)]
pub enum Mailer {
    Smtp(Arc<AsyncSmtpTransport<Tokio1Executor>>),
    /// GlobalSetting.smtp_address unset: Rails delivers with sendmail.
    Sendmail,
    /// Kept for tests and recordings.
    Memory(Arc<Mutex<Vec<Message>>>),
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mailer::Smtp(_) => f.write_str("Mailer::Smtp"),
            Mailer::Sendmail => f.write_str("Mailer::Sendmail"),
            Mailer::Memory(_) => f.write_str("Mailer::Memory"),
        }
    }
}

impl Mailer {
    pub fn memory() -> Mailer {
        Mailer::Memory(Arc::new(Mutex::new(Vec::new())))
    }

    /// The messages a memory mailer has kept.
    pub fn sent(&self) -> Vec<Message> {
        match self {
            Mailer::Memory(m) => m.lock().expect("mailer lock").clone(),
            _ => Vec::new(),
        }
    }

    /// GlobalSetting.smtp_settings as a transport.
    pub fn from_globals(globals: &GlobalSettings) -> Result<Mailer, String> {
        let Some(address) = globals.get("smtp_address").filter(|a| !a.is_empty()) else {
            return Ok(Mailer::Sendmail);
        };
        let setting = |name: &str| globals.get(name).filter(|v| !v.is_empty());
        let flag = |name: &str, default: bool| match setting(name) {
            Some(v) => matches!(v, "true" | "1" | "yes"),
            None => default,
        };
        let port: u16 = setting("smtp_port")
            .unwrap_or("25")
            .parse()
            .map_err(|_| "DISCOURSE_SMTP_PORT is not a port".to_string())?;
        let force_tls = flag("smtp_force_tls", false);
        let starttls = !force_tls && flag("smtp_enable_start_tls", true);
        let verify = setting("smtp_openssl_verify_mode") != Some("none");
        let tls_parameters = TlsParameters::builder(address.to_string())
            .dangerous_accept_invalid_certs(!verify)
            .build()
            .map_err(|e| format!("SMTP TLS: {e}"))?;
        let tls = if force_tls {
            Tls::Wrapper(tls_parameters)
        } else if starttls {
            Tls::Opportunistic(tls_parameters)
        } else {
            Tls::None
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(address)
            .port(port)
            .tls(tls)
            .timeout(Some(Duration::from_secs_f64(
                setting("smtp_read_timeout")
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(30.0),
            )));
        if let Some(domain) = setting("smtp_domain") {
            builder = builder.hello_name(lettre::transport::smtp::extension::ClientId::Domain(
                domain.to_string(),
            ));
        }
        let user = setting("smtp_user_name");
        let password = setting("smtp_password");
        if user.is_some() || password.is_some() {
            let mechanism = match setting("smtp_authentication").unwrap_or("plain") {
                "plain" => Mechanism::Plain,
                "login" => Mechanism::Login,
                other => return Err(format!("SMTP authentication {other} is not supported")),
            };
            builder = builder
                .credentials(Credentials::new(
                    user.unwrap_or_default().to_string(),
                    password.unwrap_or_default().to_string(),
                ))
                .authentication(vec![mechanism]);
        }
        Ok(Mailer::Smtp(Arc::new(builder.build())))
    }

    /// `message.deliver!`; the SMTP response, as email_logs keeps it.
    pub async fn deliver(&self, message: &Message) -> Result<Option<String>, AppError> {
        match self {
            Mailer::Memory(m) => {
                m.lock().expect("mailer lock").push(message.clone());
                Ok(None)
            }
            Mailer::Sendmail => {
                Err(Unsupported("delivering mail with sendmail (no DISCOURSE_SMTP_ADDRESS)").into())
            }
            Mailer::Smtp(transport) => {
                let encoded = encode(message)?;
                let response = transport
                    .send(encoded)
                    .await
                    .map_err(|e| SendError(e.to_string()))?;
                Ok(Some(response.message().collect::<Vec<_>>().join(" ")))
            }
        }
    }
}

#[derive(Debug)]
struct SendError(String);

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sending mail: {}", self.0)
    }
}

impl std::error::Error for SendError {}

/// The message as MIME: multipart/alternative with the text and HTML parts.
fn encode(message: &Message) -> Result<lettre::Message, AppError> {
    let mut builder = lettre::Message::builder();
    let mut raw_headers = Vec::new();
    for (name, value) in &message.headers {
        match name.to_ascii_lowercase().as_str() {
            "from" => {
                builder = builder.from(
                    value
                        .parse()
                        .map_err(|_| Unsupported("an unparseable From"))?,
                )
            }
            "to" => {
                builder = builder.to(value
                    .parse()
                    .map_err(|_| Unsupported("an unparseable To"))?)
            }
            "reply-to" => {
                builder = builder.reply_to(
                    value
                        .parse()
                        .map_err(|_| Unsupported("an unparseable Reply-To"))?,
                )
            }
            "subject" => builder = builder.subject(value.clone()),
            "message-id" => builder = builder.message_id(Some(value.clone())),
            "mime-version" | "content-type" | "content-transfer-encoding" | "date" => {}
            _ => raw_headers.push((name.clone(), value.clone())),
        }
    }
    let text = SinglePart::plain(message.text.clone());
    let body = match &message.html {
        Some(html) => MultiPart::alternative()
            .singlepart(text)
            .singlepart(SinglePart::html(html.clone())),
        None => MultiPart::mixed().singlepart(text),
    };
    let mut email = builder
        .multipart(body)
        .map_err(|e| SendError(e.to_string()))?;
    for (name, value) in raw_headers {
        let name =
            HeaderName::new_from_ascii(name).map_err(|_| Unsupported("a non-ASCII header name"))?;
        email
            .headers_mut()
            .insert_raw(HeaderValue::new(name, value));
    }
    Ok(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals(vars: &[(&str, &str)]) -> GlobalSettings {
        GlobalSettings::from_vars(vars.iter().copied())
    }

    #[test]
    fn smtp_settings_choose_the_transport() {
        assert!(matches!(
            Mailer::from_globals(&globals(&[])),
            Ok(Mailer::Sendmail)
        ));
        assert!(matches!(
            Mailer::from_globals(&globals(&[
                ("smtp_address", "smtp.example.com"),
                ("smtp_port", "587")
            ])),
            Ok(Mailer::Smtp(_))
        ));
        assert!(
            Mailer::from_globals(&globals(&[
                ("smtp_address", "smtp.example.com"),
                ("smtp_user_name", "u"),
                ("smtp_authentication", "cram_md5"),
            ]))
            .is_err()
        );
        assert!(
            Mailer::from_globals(&globals(&[("smtp_address", "x"), ("smtp_port", "nope")]))
                .is_err()
        );
    }
}
