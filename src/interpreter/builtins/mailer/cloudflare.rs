//! Delivery through Cloudflare Email Sending (`delivery_method: "cloudflare"`).
//!
//! In a Worker (the edge build) the mail goes through the `send_email`
//! binding, `env.EMAIL.send({...})`, by way of the host's `soli_email` import
//! (`edge/js/jspi.js`): no key to keep, Cloudflare knows the account. Anywhere
//! else it is a `POST` to the REST API with an API token
//! (`/accounts/{account_id}/email/sending/send`).
//!
//! Both take the same message, with two differences the docs spell out: the
//! binding reads `replyTo` and named addresses as `{email, name}`; REST reads
//! `reply_to` and addresses as strings, passed as written.
//!
//! On the Workers Free plan Cloudflare sends only to the account's verified
//! destination addresses; Workers Paid sends to anyone. The sending domain
//! must use Cloudflare DNS and be onboarded in Email Service.

use serde_json::{json, Map, Value as Json};

use super::{parse_address, MailerConfig};

/// Where the REST API lives; the account and `email/sending/send` follow.
const REST_BASE: &str = "https://api.cloudflare.com/client/v4/accounts";

/// Cloudflare's limits on one message (Email Service → Limits).
const MAX_RECIPIENTS: usize = 50;
const MAX_ATTACHMENTS: usize = 32;

/// One address as the target reads it: the binding takes `{email, name}`
/// for a named one, REST the string as written.
fn address(raw: &str, rest: bool) -> Json {
    if rest {
        return Json::String(raw.trim().to_string());
    }
    match parse_address(raw) {
        (Some(name), email) => json!({ "email": email, "name": name }),
        (None, email) => Json::String(email),
    }
}

/// Cloudflare's message, from the mailer's standard envelope (`envelope` in
/// `mailer.rs`). `rest` picks the REST spelling over the binding's.
pub(super) fn message(envelope: &Json, rest: bool) -> Result<Json, String> {
    let mut out = Map::new();
    let mut recipients = 0;
    for key in ["to", "cc", "bcc"] {
        let list: Vec<Json> = envelope[key]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Json::as_str)
                    .map(|raw| address(raw, rest))
                    .collect()
            })
            .unwrap_or_default();
        recipients += list.len();
        if !list.is_empty() {
            out.insert(key.to_string(), Json::Array(list));
        }
    }
    if recipients == 0 {
        return Err("Cloudflare Email Sending needs a recipient (to, cc or bcc)".to_string());
    }
    if recipients > MAX_RECIPIENTS {
        return Err(format!(
            "Cloudflare Email Sending takes at most {MAX_RECIPIENTS} recipients per mail \
             (to, cc and bcc together), this one has {recipients}"
        ));
    }
    let Some(from) = envelope["from"].as_str() else {
        return Err(
            "Cloudflare Email Sending needs a from address: set `from` in the mail \
                    or Mailer.configure({\"from\": …})"
                .to_string(),
        );
    };
    out.insert("from".to_string(), address(from, rest));
    out.insert("subject".to_string(), envelope["subject"].clone());
    let mut has_body = false;
    for key in ["text", "html"] {
        if let Some(body) = envelope[key].as_str() {
            has_body = true;
            out.insert(key.to_string(), Json::String(body.to_string()));
        }
    }
    if !has_body {
        return Err("Cloudflare Email Sending needs a text or an html body".to_string());
    }
    if let Some(reply_to) = envelope["reply_to"].as_str() {
        let key = if rest { "reply_to" } else { "replyTo" };
        out.insert(key.to_string(), address(reply_to, rest));
    }
    if let Some(headers) = envelope["headers"].as_object() {
        if !headers.is_empty() {
            out.insert("headers".to_string(), Json::Object(headers.clone()));
        }
    }
    if let Some(attachments) = envelope["attachments"].as_array() {
        if attachments.len() > MAX_ATTACHMENTS {
            return Err(format!(
                "Cloudflare Email Sending takes at most {MAX_ATTACHMENTS} attachments per mail, \
                 this one has {}",
                attachments.len()
            ));
        }
        if !attachments.is_empty() {
            let attachments = attachments
                .iter()
                .map(|att| {
                    json!({
                        "content": att["content_base64"],
                        "filename": att["filename"],
                        "type": att["content_type"],
                        "disposition": "attachment",
                    })
                })
                .collect();
            out.insert("attachments".to_string(), Json::Array(attachments));
        }
    }
    Ok(Json::Object(out))
}

/// Cloudflare's refusal, with what to do about the ones a setup causes.
pub(super) fn explain(code: &str, message: &str) -> String {
    let hint = match code {
        "E_RECIPIENT_NOT_ALLOWED" | "E_RECIPIENT_PLAN_RESTRICTED" => {
            " — on the Workers Free plan Cloudflare sends only to the account's verified \
             destination addresses (Workers Paid sends to anyone), and a send_email binding \
             restricted with destination_address / allowed_destination_addresses only to those"
        }
        "E_SENDER_NOT_VERIFIED"
        | "E_SENDER_DOMAIN_NOT_AVAILABLE"
        | "E_SENDER_DOMAIN_NOT_CONFIGURED"
        | "E_EMAIL_SENDING_NOT_READY" => {
            " — the from address must be on a domain onboarded in Email Service \
             (Cloudflare dashboard → Email Service → Email Sending), with its DNS records in place"
        }
        "E_HEADER_NOT_ALLOWED" | "E_HEADER_USE_API_FIELD" => {
            " — Cloudflare accepts X- headers and a short allowlist (List-Unsubscribe, \
             In-Reply-To, References…) in `headers`"
        }
        _ => "",
    };
    let code = if code.is_empty() {
        String::new()
    } else {
        format!(" ({code})")
    };
    format!("Cloudflare Email Sending refused the mail{code}: {message}{hint}")
}

/// Send `envelope` through Cloudflare: the binding in a Worker.
#[cfg(target_arch = "wasm32")]
pub(super) fn deliver(cfg: &MailerConfig, envelope: &Json) -> Result<(), String> {
    let request = json!({ "binding": cfg.binding, "message": message(envelope, false)? });
    host::send(&request.to_string())
}

/// Send `envelope` through Cloudflare: the REST API off the edge.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn deliver(cfg: &MailerConfig, envelope: &Json) -> Result<(), String> {
    let (Some(account_id), Some(token)) = (&cfg.account_id, &cfg.api_key) else {
        return Err(
            "Mailer is not configured: outside a Worker, the cloudflare delivery method \
             calls the REST API and needs account_id and api_key (an API token allowed \
             Email Sending: Edit) — Mailer.configure({\"account_id\": …, \"api_key\": …}) \
             or SOLI_MAIL_ACCOUNT_ID and SOLI_MAIL_API_KEY. In a Worker the send_email \
             binding is used instead"
                .to_string(),
        );
    };
    let url = rest_url(account_id)?;
    let headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
        ("Authorization".to_string(), format!("Bearer {token}")),
    ];
    super::send_http(&url, &headers, message(envelope, true)?.to_string())
        .map_err(|error| explain(&rest_code(&error), &error))
}

/// The binding's code for a REST refusal: REST names it in its message
/// (`email.sending.error.recipient_plan_restricted`), the binding in a code
/// (`E_RECIPIENT_PLAN_RESTRICTED`). Empty when the answer names none.
fn rest_code(error: &str) -> String {
    const PREFIX: &str = "email.sending.error.";
    let Some(at) = error.find(PREFIX) else {
        return String::new();
    };
    let name: String = error[at + PREFIX.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return String::new();
    }
    format!("E_{}", name.to_ascii_uppercase())
}

/// The REST endpoint for `account_id`, which must be an account id and not
/// a path.
fn rest_url(account_id: &str) -> Result<String, String> {
    if account_id.is_empty() || !account_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "Mailer.configure: account_id must be a Cloudflare account id (letters and digits), \
             got {account_id:?}"
        ));
    }
    Ok(format!("{REST_BASE}/{account_id}/email/sending/send"))
}

/// The binding, through the host's `soli_email` import (`edge/js/jspi.js`),
/// which the stack suspends on until Cloudflare has taken the mail.
#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "./jspi.js")]
    extern "C" {
        /// Takes the JSON request at `ptr`/`len` (`{binding, message}`);
        /// returns a buffer the host allocated with `soli_alloc`: a
        /// little-endian `u32` length, then that many bytes of JSON
        /// (`{ok, message_id}` or `{ok: false, code, error}`). Ownership passes
        /// back to Rust.
        fn soli_email(ptr: *const u8, len: usize) -> *mut u8;
    }

    pub(super) fn send(request: &str) -> Result<(), String> {
        // SAFETY: the host reads `len` bytes at `ptr` and returns a buffer
        // from `soli_alloc(4 + n)`, whose first 4 bytes are `n`.
        let response = unsafe {
            let out = soli_email(request.as_ptr(), request.len());
            if out.is_null() {
                return Err("the host returned no response".to_string());
            }
            let mut len = [0u8; 4];
            std::ptr::copy_nonoverlapping(out, len.as_mut_ptr(), 4);
            let total = 4 + u32::from_le_bytes(len) as usize;
            let buffer = Vec::from_raw_parts(out, total, total);
            buffer[4..].to_vec()
        };
        let json: serde_json::Value = serde_json::from_slice(&response)
            .map_err(|e| format!("malformed response from the host: {e}"))?;
        if json["ok"].as_bool() == Some(true) {
            return Ok(());
        }
        Err(super::explain(
            json["code"].as_str().unwrap_or(""),
            json["error"].as_str().unwrap_or("unknown error"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> Json {
        json!({
            "from": "Shop <shop@example.com>",
            "to": ["Ada Lovelace <ada@example.com>", "bob@example.com"],
            "cc": [],
            "bcc": ["audit@example.com"],
            "reply_to": "Support <help@example.com>",
            "subject": "Your order",
            "text": "Thanks",
            "html": null,
            "headers": {"X-Order": "42"},
            "attachments": [{
                "filename": "invoice.pdf",
                "content_type": "application/pdf",
                "content_base64": "JVBERi0=",
            }],
            "message_id": "abc@example.com",
            "mime": "…",
        })
    }

    #[test]
    fn the_binding_takes_named_addresses_as_objects_and_reply_to_in_camel_case() {
        let message = message(&envelope(), false).unwrap();
        assert_eq!(
            message,
            json!({
                "from": {"email": "shop@example.com", "name": "Shop"},
                "to": [{"email": "ada@example.com", "name": "Ada Lovelace"}, "bob@example.com"],
                "bcc": ["audit@example.com"],
                "replyTo": {"email": "help@example.com", "name": "Support"},
                "subject": "Your order",
                "text": "Thanks",
                "headers": {"X-Order": "42"},
                "attachments": [{
                    "content": "JVBERi0=",
                    "filename": "invoice.pdf",
                    "type": "application/pdf",
                    "disposition": "attachment",
                }],
            })
        );
    }

    #[test]
    fn rest_takes_addresses_as_written_and_reply_to_in_snake_case() {
        let message = message(&envelope(), true).unwrap();
        assert_eq!(message["from"], "Shop <shop@example.com>");
        assert_eq!(
            message["to"],
            json!(["Ada Lovelace <ada@example.com>", "bob@example.com"])
        );
        assert_eq!(message["reply_to"], "Support <help@example.com>");
        assert!(message.get("replyTo").is_none());
        assert!(message.get("cc").is_none(), "an empty list is left out");
        assert!(message.get("message_id").is_none() && message.get("mime").is_none());
    }

    #[test]
    fn a_mail_cloudflare_would_refuse_is_refused_here_first() {
        let mut no_recipient = envelope();
        no_recipient["to"] = json!([]);
        no_recipient["bcc"] = json!([]);
        assert!(message(&no_recipient, false)
            .unwrap_err()
            .contains("recipient"));

        let mut no_from = envelope();
        no_from["from"] = Json::Null;
        assert!(message(&no_from, true)
            .unwrap_err()
            .contains("from address"));

        let mut no_body = envelope();
        no_body["text"] = Json::Null;
        assert!(message(&no_body, false)
            .unwrap_err()
            .contains("text or an html"));

        let mut crowd = envelope();
        crowd["to"] = Json::Array(
            (0..51)
                .map(|i| json!(format!("u{i}@example.com")))
                .collect(),
        );
        assert!(message(&crowd, false).unwrap_err().contains("at most 50"));
    }

    #[test]
    fn a_plan_refusal_says_what_the_free_plan_allows() {
        let text = explain("E_RECIPIENT_NOT_ALLOWED", "recipient not allowed");
        assert!(text.contains("(E_RECIPIENT_NOT_ALLOWED)"));
        assert!(text.contains("verified destination addresses"));
        assert_eq!(
            explain("", "boom"),
            "Cloudflare Email Sending refused the mail: boom"
        );
    }

    #[test]
    fn a_rest_refusal_gets_the_bindings_code_and_hint() {
        let error = "mail POST https://api.cloudflare.com/client/v4/accounts/x/email/sending/send \
                     returned HTTP 403: {\"errors\":[{\"code\":10208,\"message\":\
                     \"email.sending.error.recipient_plan_restricted\"}]}";
        assert_eq!(rest_code(error), "E_RECIPIENT_PLAN_RESTRICTED");
        assert!(explain(&rest_code(error), error).contains("verified destination addresses"));
        assert_eq!(rest_code("connection refused"), "");
    }

    #[test]
    fn the_account_id_cannot_reach_another_path() {
        assert_eq!(
            rest_url("0123abcd").unwrap(),
            "https://api.cloudflare.com/client/v4/accounts/0123abcd/email/sending/send"
        );
        assert!(rest_url("abc/../../zones").is_err());
        assert!(rest_url("").is_err());
    }
}
