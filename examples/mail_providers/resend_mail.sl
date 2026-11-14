# Resend — https://resend.com/docs/api-reference/emails/send-email
#
#   Mailer.configure({"delivery_method": "http", "request": "ResendMail",
#                     "api_key": getenv("RESEND_API_KEY")})
#
# Resend takes "Name <email>" strings as they are; the key goes as a Bearer
# token, which is already how the standard request sends it.
class ResendMail
  static def request(mail, req)
    envelope = req["body"]
    base_url = req["url"].blank? ? "https://api.resend.com" : req["url"]
    body = {"from": envelope["from"], "to": envelope["to"], "subject": envelope["subject"]}
    body["cc"] = envelope["cc"] if envelope["cc"].length > 0
    body["bcc"] = envelope["bcc"] if envelope["bcc"].length > 0
    body["reply_to"] = envelope["reply_to"] unless envelope["reply_to"].nil?
    body["text"] = envelope["text"] unless envelope["text"].nil?
    body["html"] = envelope["html"] unless envelope["html"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachments"] = envelope["attachments"].map { |a|
        {"filename": a["filename"], "content": a["content_base64"]}
      }
    end
    {
      "url": base_url + "/emails",
      "headers": {"Authorization": req["headers"]["Authorization"], "Content-Type": "application/json"},
      "body": body
    }
  end
end
