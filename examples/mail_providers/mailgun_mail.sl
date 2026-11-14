# Mailgun — https://documentation.mailgun.com/docs/mailgun/api-reference/send/mailgun/messages
#
#   Mailer.configure({"delivery_method": "http", "request": "MailgunMail",
#                     "api_key": getenv("MAILGUN_API_KEY")})
#
# Sends the MIME message Soli already built (`messages.mime`), so attachments,
# inline parts and headers go through untouched. The sending domain comes from
# MAILGUN_DOMAIN, else from the From address. EU domains: add
# "url": "https://api.eu.mailgun.net".
class MailgunMail
  static def request(mail, req)
    envelope = req["body"]
    api_key = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.mailgun.net" : req["url"]
    domain = getenv("MAILGUN_DOMAIN") || envelope["from"].split("@").last.replace(">", "").trim
    boundary = "soli-mailgun-" + envelope["message_id"].to_s.split("@").first
    # Every recipient is a `to` field: the MIME has no Bcc header, and Mailgun
    # delivers to the fields, not the headers.
    recipients = [...envelope["to"], ...envelope["cc"], ...envelope["bcc"]]
    parts = recipients.map { |a| MailgunMail.field(boundary, "to", a) }
    parts.push("--" + boundary + "\r\n" +
      "Content-Disposition: form-data; name=\"message\"; filename=\"message.mime\"\r\n" +
      "Content-Type: message/rfc822\r\n\r\n" + envelope["mime"] + "\r\n")
    {
      "url": base_url + "/v3/" + domain + "/messages.mime",
      "headers": {
        "Authorization": "Basic " + Base64.encode("api:" + api_key),
        "Content-Type": "multipart/form-data; boundary=" + boundary
      },
      "body": parts.join("") + "--" + boundary + "--\r\n"
    }
  end

  static def field(boundary, name, value)
    "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + name + "\"\r\n\r\n" + value + "\r\n"
  end
end
