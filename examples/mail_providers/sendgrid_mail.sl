# SendGrid — https://www.twilio.com/docs/sendgrid/api-reference/mail-send/mail-send
#
#   Mailer.configure({"delivery_method": "http", "request": "SendgridMail",
#                     "api_key": getenv("SENDGRID_API_KEY")})
#
# EU regional subusers: add "url": "https://api.eu.sendgrid.com".
class SendgridMail
  static def request(mail, req)
    envelope = req["body"]
    base_url = req["url"].blank? ? "https://api.sendgrid.com" : req["url"]
    recipients = {"to": envelope["to"].map { |a| SendgridMail.address(a) }}
    recipients["cc"] = envelope["cc"].map { |a| SendgridMail.address(a) } if envelope["cc"].length > 0
    recipients["bcc"] = envelope["bcc"].map { |a| SendgridMail.address(a) } if envelope["bcc"].length > 0
    content = []
    content.push({"type": "text/plain", "value": envelope["text"]}) unless envelope["text"].nil?
    content.push({"type": "text/html", "value": envelope["html"]}) unless envelope["html"].nil?
    body = {
      "personalizations": [recipients],
      "from": SendgridMail.address(envelope["from"]),
      "subject": envelope["subject"],
      "content": content
    }
    body["reply_to"] = SendgridMail.address(envelope["reply_to"]) unless envelope["reply_to"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachments"] = envelope["attachments"].map { |a|
        {"content": a["content_base64"], "filename": a["filename"], "type": a["content_type"]}
      }
    end
    {
      "url": base_url + "/v3/mail/send",
      "headers": {"Authorization": req["headers"]["Authorization"], "Content-Type": "application/json"},
      "body": body
    }
  end

  # "Ana Vieira <ana@example.com>" → {"name": "Ana Vieira", "email": "ana@example.com"}
  static def address(text)
    parts = text.split("<")
    return {"email": text.trim} if parts.length < 2

    name = parts[0].trim.replace("\"", "")
    email = parts[1].replace(">", "").trim
    name.blank? ? {"email": email} : {"name": name, "email": email}
  end
end
