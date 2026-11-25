# Session, Cookie, Channel, Path, Bytes, Zip, Policy and Component.
#
# The first three are the class form of the free functions. The functions
# still run and still name themselves in errors. Path, Bytes and Zip are new.
# Policy is the deny-by-default base `soli generate auth` extends.

class PostPolicy < Policy
  def show?
    true
  end
end

class ArticlePolicy < Policy
  def create?
    true
  end
end

class MemberPolicy < Policy
  new(user)
    @user = user
  end
end

class StatsCardComponent < Component
end

describe("Session") do
  # A live, empty session whatever ran before this file.
  before_each() do
    Session.destroy
    Session.regenerate
  end

  test("stores a value and a flash under the class") do
    Session.set("framework_class_user", 7)
    assert_eq(Session.get("framework_class_user"), 7)
    assert(Session.has("framework_class_user"))

    Session.flash("notice", "saved")
    notice = Session.flashes[0]
    assert_eq(notice.type, "notice")
    assert_eq(notice.message, "saved")
  end

  test("the class and the alias name themselves") do
    assert_raises("Session.get()") do
      Session.get(1)
    end

    assert_raises("session_get()") do
      session_get(1)
    end
  end
end

describe("Cookie") do
  test("a missing sealed cookie is nil before a secret is required") do
    assert_null(Cookie.signed("framework_class_missing"))
    assert_null(Cookie.encrypted("framework_class_missing"))
    assert_null(Cookie.read("framework_class_missing"))
  end

  test("signed and encrypted round-trip through the class") do
    Session.configure({"secret": "spec-secret-0123456789abcdef-0123"})
    Cookie.signed("framework_class_uid", 42)
    assert_eq(Cookie.signed("framework_class_uid"), 42)
    Cookie.encrypted("framework_class_prefs", {"theme": "dark"})
    assert_eq(Cookie.encrypted("framework_class_prefs").theme, "dark")
  end

  test("the class and the alias name themselves") do
    assert_raises("Cookie.set()") do
      Cookie.set()
    end

    assert_raises("set_cookie()") do
      set_cookie()
    end
  end
end

describe("Channel") do
  test("join names the class outside a socket") do
    assert_raises("Channel.join()") do
      Channel.join("room")
    end

    assert_raises("ws_join()") do
      ws_join("room")
    end
  end
end

describe("Path") do
  test("joins and splits a slash path") do
    assert_eq(Path.join("app", "models", "post.sl"), "app/models/post.sl")
    assert_eq(Path.join("/var/", "/app"), "/var/app")
    assert_eq(Path.join("a//b"), "a/b")
    assert_eq(Path.dirname("app/models/post.sl"), "app/models")
    assert_eq(Path.dirname("post.sl"), ".")
    assert_eq(Path.dirname("/a"), "/")
    assert_eq(Path.dirname("/"), "/")
    assert_eq(Path.basename("app/models/post.sl"), "post.sl")
    assert_eq(Path.basename("/"), "/")
    assert_eq(Path.extname("archive.tar.gz"), ".gz")
    assert_eq(Path.extname(".gitignore"), "")
  end

  test("join requires a path") do
    assert_raises("Path.join()") do
      Path.join()
    end
  end
end

describe("Bytes") do
  test("round-trips text, base64 and a byte array") do
    hello = Bytes.from_string("hello")
    assert_eq(hello.length, 5)
    assert_eq(hello.to_base64, "aGVsbG8=")
    assert_eq(hello.slice(1, 4).to_string, "ell")
    assert_eq(hello.slice(5, 5).length, 0)
    assert_eq(Bytes.from_array([104, 105]).to_string, "hi")
    assert_eq(Bytes.from_string("").length, 0)
    assert_eq(Bytes.from_base64("aGVsbG8=").to_string, "hello")
  end

  test("rejects a bad range, bad base64 and non-UTF-8") do
    hello = Bytes.from_string("hello")

    assert_raises("Bytes.slice()") do
      hello.slice(3, 1)
    end

    assert_raises("Bytes.slice()") do
      hello.slice(0, 99)
    end

    assert_raises("Bytes.slice()") do
      hello.slice(-1)
    end

    assert_raises("Bytes.from_base64()") do
      Bytes.from_base64("@@@")
    end

    assert_raises("Bytes.to_string()") do
      Bytes.from_array([255]).to_string
    end
  end
end

describe("Zip") do
  test("writes and reads text and base64 entries") do
    archive = Zip.create({
      "hello.txt": "hi",
      "bin.dat": {"base64": "YWJj"}
    })
    assert_eq(Zip.read_text(archive, "hello.txt"), "hi")
    assert_eq(Zip.read(archive, "bin.dat"), "YWJj")
    assert_eq(Zip.entries(archive).length, 2)
    names = Zip.entries(archive).map { |entry| entry.name }
    assert(names.includes?("hello.txt"))
    assert(names.includes?("bin.dat"))
  end

  test("refuses a name that climbs out of the archive") do
    assert_raises("Zip.create()") do
      Zip.create({"../secret": "no"})
    end
  end
end

describe("Policy") do
  test("denies until a subclass allows the action") do
    post = PostPolicy.new
    assert(post.show?)
    assert(!post.index?)
    assert(ArticlePolicy.new.new?)
    assert(!ArticlePolicy.new.edit?)
    assert(!Policy.new.signed_in?)
    assert(MemberPolicy.new({"name": "ada"}).signed_in?)
    assert(!MemberPolicy.new(nil).signed_in?)
  end
end

describe("Component") do
  test("a generated-style class is a Component") do
    card = StatsCardComponent.new
    assert(card.is_a?("StatsCardComponent"))
    assert(card.is_a?("Component"))
  end
end
