# Postmark — https://postmarkapp.com/developer/api/email-api
#
#   Mailer.configure({"delivery_method": "http", "request": "PostmarkMail",
#                     "api_key": getenv("POSTMARK_SERVER_TOKEN")})
#
# `From` must be a confirmed Sender Signature on your Postmark server.
class PostmarkMail
  static def request(mail, req)
    envelope = req["body"]
    token = req["headers"]["Authorization"].to_s.replace("Bearer ", "")
    base_url = req["url"].blank? ? "https://api.postmarkapp.com" : req["url"]
    body = {"From": envelope["from"], "To": envelope["to"].join(", "), "Subject": envelope["subject"]}
    body["Cc"] = envelope["cc"].join(", ") if envelope["cc"].length > 0
    body["Bcc"] = envelope["bcc"].join(", ") if envelope["bcc"].length > 0
    body["ReplyTo"] = envelope["reply_to"] unless envelope["reply_to"].nil?
    body["TextBody"] = envelope["text"] unless envelope["text"].nil?
    body["HtmlBody"] = envelope["html"] unless envelope["html"].nil?
    if envelope["headers"].length > 0
      body["Headers"] = envelope["headers"].keys.map { |name| {"Name": name, "Value": envelope["headers"][name]} }
    end
    if envelope["attachments"].length > 0
      body["Attachments"] = envelope["attachments"].map { |a|
        {"Name": a["filename"], "Content": a["content_base64"], "ContentType": a["content_type"]}
      }
    end
    {
      "url": base_url + "/email",
      "headers": {
        "X-Postmark-Server-Token": token,
        "Content-Type": "application/json",
        "Accept": "application/json"
      },
      "body": body
    }
  end
end
