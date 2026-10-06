# Mailer: the full dispatch (UserMailer.welcome(user) -> method_missing ->
# action -> @mail(...) -> Message) and the `test` delivery mode, which captures
# mail in Mailer.deliveries() instead of sending it.

class UserMailer < Mailer
  def welcome(user)
    @user = user
    @mail(to: user["email"], subject: "Welcome, #{user["name"]}", html: "<p>Hi #{user["name"]}</p>")
  end

  def announcement(user)
    @mail(
      to: [user["email"], "team@example.com"],
      cc: "cc@example.com",
      bcc: "audit@example.com",
      reply_to: "support@example.com",
      sender: "Boss <boss@example.com>",
      subject: "News",
      text: "plain body",
      html: "<b>html body</b>"
    )
  end
end

class OrderMailer < Mailer
  # Multi-argument action (order + invoice).
  def receipt(order, invoice)
    @order = order
    @mail(to: order["email"], subject: "Receipt #{invoice}", html: "<p>Order #{order["id"]}</p>")
  end
end

ALICE = {"email": "alice@example.com", "name": "Alice"}

describe("Mailer") do
  before_each() do
    Mailer.configure({"delivery_method": "test", "from": "noreply@example.com"})
    Mailer.clear_deliveries()
  end

  describe("building a message") do
    test("an action renders to the hash it will send") do
      assert_eq(UserMailer.welcome(ALICE).to_h, {
        "to": "alice@example.com",
        "subject": "Welcome, Alice",
        "html": "<p>Hi Alice</p>",
        "from": "noreply@example.com"
      })
    end

    test("building a message does not deliver it") do
      UserMailer.welcome(ALICE)
      assert_eq(Mailer.deliveries(), [])
    end

    test("every mail() argument lands in the hash, and sender overrides From") do
      mail = UserMailer.announcement(ALICE).to_h
      assert_eq(mail["to"], ["alice@example.com", "team@example.com"])
      assert_eq(mail["cc"], "cc@example.com")
      assert_eq(mail["bcc"], "audit@example.com")
      assert_eq(mail["reply_to"], "support@example.com")
      assert_eq(mail["from"], "Boss <boss@example.com>")
      assert_eq(mail["text"], "plain body")
      assert_eq(mail["html"], "<b>html body</b>")
    end

    test("dispatches a multi-argument action") do
      OrderMailer.receipt({"email": "buyer@example.com", "id": 42}, "INV-7").deliver_now
      sent = Mailer.deliveries()
      assert_eq(sent[0]["to"], "buyer@example.com")
      assert_eq(sent[0]["subject"], "Receipt INV-7")
      assert_eq(sent[0]["html"], "<p>Order 42</p>")
    end

    test("an undefined action raises with its name") do
      assert_raises("mailer action 'nope' is not defined") do
        UserMailer.nope(ALICE)
      end
    end
  end

  describe("attachments") do
    test("attach() and attach_base64() chain and append in order") do
      message = UserMailer.welcome({"email": "a@b.c", "name": "A"})
      message.attach("notes.txt", "thanks").attach_base64("logo.png", "aGVsbG8=", "image/png")
      attachments = message.to_h["attachments"]
      assert_eq(attachments.length, 2)
      assert_eq(attachments[0]["filename"], "notes.txt")
      assert_eq(attachments[0]["content"], "thanks")
      assert_eq(attachments[1]["filename"], "logo.png")
      assert_eq(attachments[1]["base64"], "aGVsbG8=")
      assert_eq(attachments[1]["content_type"], "image/png")
    end
  end

  describe("test delivery mode") do
    test("deliver_now captures the rendered mail") do
      UserMailer.welcome(ALICE).deliver_now
      sent = Mailer.deliveries()
      assert_eq(sent.length, 1)
      assert_eq(sent[0]["to"], "alice@example.com")
      assert_eq(sent[0]["subject"], "Welcome, Alice")
      assert_eq(sent[0]["html"], "<p>Hi Alice</p>")
    end

    test("deliver_later also captures in test mode") do
      UserMailer.welcome({"email": "bob@example.com", "name": "Bob"}).deliver_later
      assert_eq(Mailer.deliveries().length, 1)
      assert_eq(Mailer.deliveries()[0]["to"], "bob@example.com")
    end

    test("deliveries are kept newest last") do
      UserMailer.welcome({"email": "first@example.com", "name": "First"}).deliver_now
      UserMailer.welcome({"email": "second@example.com", "name": "Second"}).deliver_now
      assert_eq(Mailer.deliveries().map { |mail| mail["to"] }, ["first@example.com", "second@example.com"])
    end

    test("clear_deliveries empties the capture buffer") do
      UserMailer.welcome({"email": "a@b.c", "name": "A"}).deliver_now
      assert_eq(Mailer.deliveries().length, 1)
      Mailer.clear_deliveries()
      assert_eq(Mailer.deliveries(), [])
    end

    test("Mailer.deliver captures a hand-built hash with its headers") do
      Mailer.deliver({
        "to": "a@b.c",
        "subject": "Direct",
        "text": "t",
        "headers": {"In-Reply-To": "<one@mail>"}
      })
      sent = Mailer.deliveries()
      assert_eq(sent.length, 1)
      assert_eq(sent[0]["subject"], "Direct")
      assert_eq(sent[0]["headers"], {"In-Reply-To": "<one@mail>"})
    end
  end
end
