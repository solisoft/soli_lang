# Xml: building and mutating trees — Xml.document, Xml.element, node.add
# (with named attributes and with a block), Xml.build, the mutation methods,
# moving versus copying, validation and escaping.

describe("Xml building") do
  describe("constructors") do
    test("Xml.document writes a declaration") do
      doc = Xml.document("rss", version: "2.0")
      assert_eq(type(doc), "XmlDocument")
      assert_eq(doc.to_xml, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\"/>")
      assert_eq(doc.root.to_xml, "<rss version=\"2.0\"/>")
    end

    test("Xml.document with no root") do
      doc = Xml.document
      assert_null(doc.root)
    end

    test("Xml.element builds a detached node") do
      el = Xml.element("a", ["t", Xml.element("b")], {"x": 1})
      assert_eq(el.to_xml, "<a x=\"1\">t<b/></a>")
      assert_null(el.parent)
      assert_null(el.document)
    end
  end

  describe("add") do
    test("returns the child, with content and attributes") do
      root = Xml.document("r").root
      child = root.add("item", "News & views", id: 7)
      assert_eq(child.name, "item")
      assert_eq(child.to_xml, "<item id=\"7\">News &amp; views</item>")
      assert(child.parent == root)
    end

    test("a Hash in the content position is the attributes") do
      root = Xml.document("r").root
      assert_eq(root.add("a", {"k": "v"}).to_xml, "<a k=\"v\"/>")
      assert_eq(root.add("b", id: 1, gone: nil).to_xml, "<b id=\"1\"/>")
    end

    test("scalars are written with to_s") do
      root = Xml.document("r").root
      root.add("i", 3)
      root.add("f", 1.5)
      root.add("b", true)
      assert_eq(root.to_xml, "<r><i>3</i><f>1.5</f><b>true</b></r>")
    end

    test("the leaf kinds") do
      root = Xml.document("r").root
      root.add_text("a < b")
      root.add_cdata("x]]>y")
      root.add_comment("note")
      root.add_pi("go", "now")
      assert_eq(root.to_xml, "<r>a &lt; b<![CDATA[x]]]]><![CDATA[>y]]><!--note--><?go now?></r>")
    end

    test("refuses an invalid name and a bad comment") do
      root = Xml.document("r").root
      assert_raises("XmlNode.add") do
        root.add("1bad")
      end
      assert_raises("XmlNode.add_comment") do
        root.add_comment("a--b")
      end
    end

    test("drops the characters XML forbids") do
      root = Xml.document("r").root
      root.add("t", "a\u0001b", k: "c\u0002d")
      assert_eq(root.to_xml, "<r><t k=\"cd\">ab</t></r>")
    end

    test("resolves prefixes against the scope") do
      root = Xml.document("r", {"xmlns:p": "urn:p"}).root
      assert_eq(root.add("p:x").namespace, "urn:p")
      assert_raises("XmlNode.add") do
        root.add("q:x")
      end
    end
  end

  describe("the block builder") do
    test("Xml.build gives the root to the block and returns the document") do
      doc = Xml.build("feed", {"xmlns": "http://www.w3.org/2005/Atom"}) do |feed|
        feed.add("title", "T & U")
        [1, 2].each do |i|
          feed.add("entry", id: i) do |entry|
            entry.add("n", i * 10)
          end
        end
      end
      assert_eq(type(doc), "XmlDocument")
      expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>T &amp; U</title><entry id=\"1\"><n>10</n></entry><entry id=\"2\"><n>20</n></entry></feed>"
      assert_eq(doc.to_xml, expected)
    end

    test("add with a block returns the child") do
      root = Xml.document("r").root
      child = root.add("block", nil, {"k": "v"}) do |b|
        b.add("inner", "in")
        b["at"] = "1"
      end
      assert_eq(child.name, "block")
      assert_eq(child.to_xml, "<block k=\"v\" at=\"1\"><inner>in</inner></block>")
    end

    test("a lambda passed with & or positionally is the block") do
      root = Xml.document("r").root
      blk = fn(node) { node.add("by_ref") }
      root.add("a", &blk)
      root.add("b", fn(node) { node.add("positional") })
      assert_eq(root.to_xml, "<r><a><by_ref/></a><b><positional/></b></r>")
    end

    test("the block sees and updates outer variables") do
      count = 0
      Xml.build("r") do |r|
        [1, 2, 3].each do |i|
          r.add("i") do |c|
            count = count + 1
          end
        end
      end
      assert_eq(count, 3)
    end

    test("an error in the block propagates") do
      assert_raises("boom") do
        Xml.build("r") do |r|
          r.add("a") do |a|
            throw "boom"
          end
        end
      end
    end

    test("Xml.build needs a root name") do
      assert_raises("Xml.build") do
        Xml.build(nil) do |r|
          r.add("x")
        end
      end
    end
  end

  describe("mutation") do
    test("attributes by index, text= and name=") do
      root = Xml.parse("<a id=\"1\"><b>old</b></a>").root
      root["id"] = 2
      root["new"] = "n"
      root["gone"] = nil
      assert_eq(root.remove_attribute("new"), "n")
      assert_null(root.remove_attribute("new"))
      b = root.children[0]
      b.text = "fresh & new"
      b.name = "c"
      assert_eq(root.to_xml, "<a id=\"2\"><c>fresh &amp; new</c></a>")
      assert_raises("XmlNode.name=") do
        b.name = "not a name"
      end
    end

    test("append, prepend, insert_before, insert_after, replace") do
      root = Xml.parse("<r><m/></r>").root
      m = root.children[0]
      root.append("tail")
      root.prepend(Xml.element("first"))
      m.insert_before(Xml.element("before"))
      m.insert_after(Xml.element("after"))
      assert_eq(root.to_xml, "<r><first/><before/><m/><after/>tail</r>")
      m.replace(Xml.element("swapped"))
      assert_eq(root.to_xml, "<r><first/><before/><swapped/><after/>tail</r>")
    end

    test("a node of the same tree moves, a node of another tree is copied") do
      doc = Xml.parse("<r><a><x/></a><b/></r>")
      a = doc.root.children[0]
      b = doc.root.children[1]
      x = a.children[0]
      b.append(x)
      # A parsed <a><x/></a> keeps its expanded form once emptied.
      assert_eq(doc.root.to_xml, "<r><a></a><b><x/></b></r>")
      other = Xml.element("o")
      copied = b.append(other)
      assert(copied != other)
      assert_null(other.parent)
      assert(copied.parent == b)
    end

    test("add leaves no element behind when its content fails") do
      n = Xml.element("x")
      assert_raises("would create a cycle") do
        n.add("y", [n])
      end
      assert_eq(n.to_xml, "<x/>")
    end

    test("refuses a cycle") do
      doc = Xml.parse("<r><a><b/></a></r>")
      a = doc.root.children[0]
      assert_raises("would create a cycle") do
        a.children[0].append(a)
      end
    end

    test("remove detaches and keeps the node usable") do
      doc = Xml.parse("<r><a><x/></a></r>")
      a = doc.root.children[0].remove
      assert_null(a.parent)
      assert_eq(doc.root.children.length, 0)
      a.add("y")
      assert_eq(a.to_xml, "<a><x/><y/></a>")
    end

    test("dup is a deep copy in a fresh tree") do
      doc = Xml.parse("<r xmlns:p=\"urn:p\"><p:a k=\"1\"/></r>")
      copy = doc.root.children[0].dup
      assert_null(copy.document)
      copy["k"] = "2"
      assert_eq(doc.root.children[0]["k"], "1")
      assert_eq(copy.add("p:b").namespace, "urn:p")
    end
  end

  describe("round trip") do
    test("an RSS feed built then re-parsed") do
      posts = [{"id": 1, "title": "A & B", "body": "<p>x</p>"}, {"id": 2, "title": "C", "body": "y"}]
      doc = Xml.document("rss", version: "2.0")
      channel = doc.root.add("channel")
      channel.add("title", "News")
      posts.each do |post|
        item = channel.add("item", id: post.id)
        item.add("title", post.title)
        item.add_cdata(post.body)
      end
      reparsed = Xml.parse(doc.to_xml)
      assert_eq(reparsed.to_xml, doc.to_xml)
      items = reparsed.root.children[0].children.filter { |c| c.name == "item" }
      assert_eq(items.map { |i| i["id"] }, ["1", "2"])
      assert_eq(items[0].children[0].text, "A & B")
      assert_eq(items[0].text, "A & B<p>x</p>")
      assert_eq(Xml.parse(doc.to_xml(indent: 2)).root.children[0].children.length, 3)
    end
  end
end
