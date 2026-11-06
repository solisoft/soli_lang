# Xml queries: `find` / `find_all` (lenient, ElementTree-like) and
# `xpath` / `at_xpath` (strict XPath 1.0), over one XPath subset.

const FEED = "tests/builtins/_fixtures/xml/feed.rss"
const FACTURX = "tests/builtins/_fixtures/xml/factur-x.xml"
const ATOM = "tests/builtins/_fixtures/xml/atom.xml"
const RAM = "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100"
const ATOM_NS = "http://www.w3.org/2005/Atom"

describe("Xml queries") do
  describe("find and find_all") do
    test("find on a document starts at the root element") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.find("channel/title")&.text, "News & views")
      assert_null(doc.find("rss"))
      assert_null(doc.find("channel/nope"))
      assert_eq(doc.find(".").name, "rss")
    end

    test("find_all returns nodes in document order") do
      doc = Xml.parse_file(FEED)
      items = doc.find_all("//item")
      assert_eq(items.length, 2)
      assert_eq(type(items[0]), "XmlNode")
      assert_eq(items.map { |item| item["id"] }, ["1", "2"])
      assert_eq(doc.find_all("nothing"), [])
    end

    test("@attr and text() give Strings") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.find_all("//item/@id"), ["1", "2"])
      assert_eq(doc.find_all("channel/item/title/text()"), ["First", "Second"])
      assert_eq(doc.find("//description/text()"), "<p>Hello</p>")
      assert_eq(doc.find("channel/item/@nope"), nil)
    end

    test("unprefixed names match in any namespace") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.find_all("//creator").map { |c| c.text }, ["Ada", "Grace"])
      assert_eq(doc.find("//dc:creator").text, "Ada")
    end

    test("predicates") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.find("channel/item[@id='2']/title").text, "Second")
      assert_eq(doc.find("channel/item[title='First']")["id"], "1")
      assert_eq(doc.find("channel/item[last()]")["id"], "2")
      assert_eq(doc.find("channel/item[2]")["id"], "2")
      assert_eq(doc.find_all("channel/item[position() > 1]").length, 1)
      assert_eq(doc.find_all("channel/item[not(description)]/@id"), ["2"])
      assert_eq(doc.find_all("channel/item[contains(title, 'eco') or @id='1']/@id"), ["1", "2"])
      assert_eq(doc.find_all("channel/item[starts-with(title, 'F')]/@id"), ["1"])
    end

    test("on an element, a relative path starts at the element") do
      channel = Xml.parse_file(FEED).find("channel")
      assert_eq(channel.find("item/title").text, "First")
      assert_eq(channel.find("..").kind, "element")
      assert_eq(channel.find("../..").kind, "document")
      assert_eq(channel.find_all("/rss/channel/item").length, 2)
    end

    test("a result is the same node as a navigated one") do
      doc = Xml.parse_file(FEED)
      first = doc.find("//item")
      assert(first == doc.find_all("//item")[0])
      assert(first == doc.root.children[0].children[2])
      assert(first.parent == doc.find("channel"))
    end
  end

  describe("xpath and at_xpath") do
    test("a relative path on a document starts at the document node") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.xpath("rss/channel/item").length, 2)
      assert_null(doc.at_xpath("channel"))
      assert_eq(doc.at_xpath("/rss/channel/title").text, "News & views")
      assert_eq(type(doc.at_xpath("/")), "XmlDocument")
      assert_eq(doc.at_xpath(".").kind, "document")
    end

    test("results come in document order without duplicates, across |") do
      doc = Xml.parse_file(FEED)
      names = doc.xpath("//item/title | /rss/channel/title | //item[1]/title").map { |t| t.text }
      assert_eq(names, ["News & views", "First", "Second"])
      assert_eq(doc.xpath("//item/@id | //item[2]").map { |x| type(x) }, ["string", "XmlNode", "string"])
    end

    test("[n] counts per parent, as in XPath") do
      doc = Xml.parse("<r><a><b>1</b><b>2</b></a><a><b>3</b></a></r>")
      assert_eq(doc.xpath("//b[1]/text()"), ["1", "3"])
      assert_eq(doc.xpath("//a[2]/b/text()"), ["3"])
      assert_eq(doc.xpath("//b[last()]/text()"), ["2", "3"])
    end

    test("comparisons are existential string comparisons") do
      doc = Xml.parse("<r><i><t>x</t><t>y</t></i><i><t>z</t></i><i/></r>")
      assert_eq(doc.xpath("//i[t='y']").length, 1)
      assert_eq(doc.xpath("//i[t!='x']").length, 2)
      assert_eq(doc.xpath("//i[.='xy']").length, 1)
      assert_eq(doc.xpath("//i[not(t)]").length, 1)
    end

    test("text(), comment() and node()") do
      doc = Xml.parse("<a>one<!--c--><b/><![CDATA[two]]></a>")
      assert_eq(doc.xpath("/a/text()"), ["one", "two"])
      assert_eq(doc.at_xpath("/a/comment()").kind, "comment")
      assert_eq(doc.xpath("/a/node()").map { |n| n.kind }, ["text", "comment", "element", "cdata"])
    end
  end

  describe("Factur-X (CII)") do
    test("the seller, with a table") do
      doc = Xml.parse_file(FACTURX)
      seller = doc.at_xpath("//ram:SellerTradeParty/ram:Name", {"ram": RAM})
      assert_eq(seller.text, "Soli SARL")
    end

    test("named arguments are the table, any prefix works") do
      doc = Xml.parse_file(FACTURX)
      assert_eq(doc.at_xpath("//r:GrandTotalAmount/text()", r: RAM), "120.00")
    end

    test("prefixes fall back to the root's declarations") do
      doc = Xml.parse_file(FACTURX)
      lines = doc.xpath("//ram:IncludedSupplyChainTradeLineItem")
      assert_eq(lines.length, 2)
      names = lines.map { |line| line.at_xpath("ram:SpecifiedTradeProduct/ram:Name/text()") }
      assert_eq(names, ["Widget", "Gadget"])
      assert_eq(doc.at_xpath("//ram:TaxTotalAmount/@currencyID"), "EUR")
      assert_eq(doc.at_xpath("//ram:BuyerTradeParty/ram:Name").text, "ACME & Co")
      assert_eq(doc.at_xpath("//ram:ID[@schemeID='VA']/text()"), "FR12345678901")
    end

    test("find reads it without any prefix") do
      doc = Xml.parse_file(FACTURX)
      assert_eq(doc.find("//GrandTotalAmount").text, "120.00")
      assert_eq(doc.find_all("SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem//LineID/text()"), ["1", "2"])
    end
  end

  describe("Atom (a default namespace)") do
    test("find matches despite the default namespace") do
      feed = Xml.parse_file(ATOM)
      entries = feed.find_all("entry")
      assert_eq(entries.length, 2)
      assert_eq(entries[0].find("title").text, "XML lands in Soli")
      assert_eq(feed.find_all("entry/link[@rel='alternate']/@href"), ["https://example.com/posts/xml", "https://example.com/posts/xpath"])
    end

    test("xpath needs the namespace") do
      feed = Xml.parse_file(ATOM)
      assert_eq(feed.xpath("//entry"), [])
      assert_eq(feed.xpath("//xmlns:entry").length, 2)
      assert_eq(feed.xpath("//*:entry").length, 2)
      assert_eq(feed.xpath("//entry", {"": ATOM_NS}).length, 2)
      assert_eq(feed.xpath("//a:entry/a:title/text()", {"a": ATOM_NS}), ["XML lands in Soli", "XPath, the subset"])
    end

    test("the \"\" key works with find too") do
      feed = Xml.parse_file(ATOM)
      assert_eq(feed.find_all("entry", {"": ATOM_NS}).length, 2)
      assert_eq(feed.find_all("entry", {"": "urn:other"}), [])
    end

    test("other prefixes resolve through the root") do
      feed = Xml.parse_file(ATOM)
      assert_eq(feed.find("//media:thumbnail/@url"), "https://example.com/xml.png")
      assert_eq(feed.at_xpath("//media:thumbnail/@url"), "https://example.com/xml.png")
    end
  end

  describe("detached nodes and edits") do
    test("a built fragment answers absolute paths from its own top") do
      n = Xml.element("a")
      n.add("b", id: 1)
      n.add("b", id: 2)
      assert_eq(n.find_all("//b/@id"), ["1", "2"])
      assert_eq(n.at_xpath("/a/b[2]")["id"], "2")
      assert_null(n.at_xpath("/"))
      assert_null(n.at_xpath(".."))
    end

    test("queries see edits in document order") do
      doc = Xml.parse_file(FEED)
      channel = doc.find("channel")
      channel.prepend(Xml.element("item", id: "0"))
      assert_eq(doc.find_all("//item/@id"), ["0", "1", "2"])
      doc.find("//item[@id='1']").remove
      assert_eq(doc.xpath("//item/@id"), ["0", "2"])
    end
  end

  describe("errors") do
    test("unsupported XPath names the column") do
      doc = Xml.parse("<a/>")
      assert_raises("Xml.xpath: unsupported XPath at column 1: function 'sum'") do
        doc.xpath("sum(//a)")
      end
      assert_raises("Xml.find: unsupported XPath at column 1: axis 'child::'") do
        doc.find("child::a")
      end
      assert_raises("Xml.xpath: unsupported XPath at column 9: numeric comparison") do
        doc.xpath("//a[@id=1]")
      end
      assert_raises("Xml.find: invalid XPath at column 1: unterminated string literal") do
        doc.find_all("'abc")
      end
    end

    test("an unbound prefix is refused") do
      doc = Xml.parse("<a/>")
      assert_raises("Xml.xpath: unbound prefix 'q'") do
        doc.at_xpath("//q:a")
      end
      assert_raises("Xml.find: unbound prefix 'xmlns'") do
        doc.find("xmlns:a")
      end
    end

    test("bad arguments") do
      doc = Xml.parse("<a/>")
      assert_raises("Xml.xpath: the expression must be a String, got int") do
        doc.xpath(42)
      end
      assert_raises("Xml.find: the namespaces must be a Hash of prefix => URI, got string") do
        doc.find("a", "urn:x")
      end
      assert_raises("Xml.find: the URI for prefix 'p' must be a String, got int") do
        doc.find("a", {"p": 1})
      end
      assert_raises("Xml.xpath takes (expression, namespaces = nil), got 3 arguments") do
        doc.xpath("a", {}, {})
      end
      assert_raises("Xml.find: missing the expression") do
        doc.find
      end
    end

    test("expression limits") do
      doc = Xml.parse("<a/>")
      assert_raises("XPath too complex: more than 64 steps") do
        doc.xpath((1..66).map { |i| "a" }.join("/"))
      end
      assert_raises("XPath too complex: the expression is longer than 4096 bytes") do
        doc.xpath("a[@x='#{"y" * 5000}']")
      end
    end

    test("the visit budget stops a runaway query") do
      top = Xml.element("a")
      node = top
      (1..5000).each do |i|
        node = node.add("a")
      end
      node.add_text("x")
      assert_eq(top.xpath("//*//*//*").length, 4998)
      assert_raises("Xml.xpath: evaluation exceeded 10000000 node visits (SOLI_MAX_XPATH_VISITS)") do
        top.xpath("//*[.='y']")
      end
    end
  end
end
