# Brevo — https://developers.brevo.com/reference/sendtransacemail
#
#   Mailer.configure({"delivery_method": "http", "request": "BrevoMail",
#                     "api_key": getenv("BREVO_API_KEY")})
#
# A configured `url` replaces https://api.brevo.com (a proxy, a test server).
class BrevoMail
  static def request(mail, req)
    envelope = req["body"]
    api_key = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.brevo.com" : req["url"]
    body = {
      "sender": BrevoMail.address(envelope["from"]),
      "to": envelope["to"].map { |a| BrevoMail.address(a) },
      "subject": envelope["subject"]
    }
    body["cc"] = envelope["cc"].map { |a| BrevoMail.address(a) } if envelope["cc"].length > 0
    body["bcc"] = envelope["bcc"].map { |a| BrevoMail.address(a) } if envelope["bcc"].length > 0
    body["replyTo"] = BrevoMail.address(envelope["reply_to"]) unless envelope["reply_to"].nil?
    body["textContent"] = envelope["text"] unless envelope["text"].nil?
    body["htmlContent"] = envelope["html"] unless envelope["html"].nil?
    body["headers"] = envelope["headers"] if envelope["headers"].length > 0
    if envelope["attachments"].length > 0
      body["attachment"] = envelope["attachments"].map { |a|
        {"name": a["filename"], "content": a["content_base64"]}
      }
    end
    {
      "url": base_url + "/v3/smtp/email",
      "headers": {"api-key": api_key, "Content-Type": "application/json", "Accept": "application/json"},
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
