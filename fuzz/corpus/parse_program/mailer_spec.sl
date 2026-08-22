# ============================================================================
# Mailer Test Suite — exercises the full dispatch (UserMailer.welcome(user)
# -> method_missing -> action -> this.mail(...) -> Message) and the `test`
# delivery mode that captures mail in Mailer.deliveries() for assertions.
# ============================================================================

class UserMailer < Mailer {
  def welcome(user) {
    @user = user;
    return this.mail(
      to: user["email"],
      subject: "Welcome, " + user["name"],
      html: "<p>Hi " + user["name"] + "</p>"
    );
  }
}

class OrderMailer < Mailer {
  # Multi-argument action (order + invoice).
  def receipt(order, invoice) {
    @order = order;
    return this.mail(
      to: order["email"],
      subject: "Receipt " + invoice,
      html: "<p>Order " + str(order["id"]) + "</p>"
    );
  }
}

describe("Mailer", fn() {
  before_each(fn() {
    Mailer.configure({ "delivery_method": "test", "from": "noreply@example.com" });
    Mailer.clear_deliveries();
  });

  test("dispatches an action and captures the rendered mail", fn() {
    UserMailer.welcome({ "email": "alice@example.com", "name": "Alice" }).deliver_now();
    let sent = Mailer.deliveries();
    assert_eq(len(sent), 1);
    assert_eq(sent[0]["to"], "alice@example.com");
    assert_eq(sent[0]["subject"], "Welcome, Alice");
    assert_eq(sent[0]["html"], "<p>Hi Alice</p>");
  });

  test("uses the configured default From", fn() {
    let msg = UserMailer.welcome({ "email": "x@y.z", "name": "X" });
    assert_eq(msg.to_h()["from"], "noreply@example.com");
  });

  test("deliver_later also captures in test mode", fn() {
    UserMailer.welcome({ "email": "bob@example.com", "name": "Bob" }).deliver_later();
    assert_eq(len(Mailer.deliveries()), 1);
    assert_eq(Mailer.deliveries()[0]["to"], "bob@example.com");
  });

  test("clear_deliveries empties the capture buffer", fn() {
    UserMailer.welcome({ "email": "a@b.c", "name": "A" }).deliver_now();
    assert_eq(len(Mailer.deliveries()), 1);
    Mailer.clear_deliveries();
    assert_eq(len(Mailer.deliveries()), 0);
  });

  test("dispatches a multi-argument action", fn() {
    OrderMailer.receipt({ "email": "buyer@example.com", "id": 42 }, "INV-7").deliver_now();
    let sent = Mailer.deliveries();
    assert_eq(sent[0]["to"], "buyer@example.com");
    assert_eq(sent[0]["subject"], "Receipt INV-7");
    assert_eq(sent[0]["html"], "<p>Order 42</p>");
  });

  test("attach() and attach_base64() append chainable attachments", fn() {
    let msg = UserMailer.welcome({ "email": "a@b.c", "name": "A" });
    msg.attach("notes.txt", "thanks").attach_base64("logo.png", "aGVsbG8=", "image/png");
    let mail = msg.to_h();
    assert_eq(len(mail["attachments"]), 2);
    assert_eq(mail["attachments"][0]["filename"], "notes.txt");
    assert_eq(mail["attachments"][1]["content_type"], "image/png");
  });
});
