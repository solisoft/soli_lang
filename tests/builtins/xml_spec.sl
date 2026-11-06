# Xml: parsing, the node objects (identity, navigation, text, attributes),
# serialisation, `to_h`, display, JSON, equality and errors.
# Queries (find / xpath) are in xml_xpath_spec.sl, building in xml_build_spec.sl.

const FEED = "tests/builtins/_fixtures/xml/feed.rss"
const SMALL = "<rss version=\"2.0\"><channel><title>A &amp; B</title><item id=\"1\"><t>x</t></item><item id=\"2\"/></channel></rss>"

describe("Xml") do
  describe("parsing") do
    test("Xml.parse returns an XmlDocument") do
      doc = Xml.parse(SMALL)
      assert_eq(type(doc), "XmlDocument")
      assert_eq(doc.kind, "document")
      assert(doc.document?)
      assert(doc.is_a?("XmlNode"))
      assert(doc.is_a?("XmlDocument"))
      assert_eq(doc.root.name, "rss")
    end

    test("Xml.parse_file reads a file") do
      doc = Xml.parse_file(FEED)
      assert_eq(doc.root.name, "rss")
      assert_eq(doc.root.children[0].children[0].text, "News & views")
    end

    test("refuses malformed input with a line and column") do
      assert_raises("Xml.parse: line 1, column 7: expected </b>, found </a>") do
        Xml.parse("<a><b></a>")
      end
    end

    test("refuses a DOCTYPE") do
      assert_raises("DOCTYPE declarations are not allowed") do
        Xml.parse("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>")
      end
    end

    test("refuses a non-String argument") do
      assert_raises("Xml.parse: expects the XML as a String") do
        Xml.parse(42)
      end
    end

    test("refuses an unknown option") do
      assert_raises("unknown option 'deep'") do
        Xml.parse("<a/>", deep: true)
      end
    end

    test("options can only lower limits") do
      assert_raises("Xml.parse") do
        Xml.parse("<a><b><c/></b></a>", max_depth: 2)
      end
      assert_eq(Xml.parse("<a><b/></a>", max_depth: 2).root.name, "a")
    end

    test("strip_blank drops whitespace-only text") do
      source = "<a>\n  <b/>\n</a>"
      assert_eq(Xml.parse(source).root.nodes.length, 3)
      assert_eq(Xml.parse(source, strip_blank: true).root.nodes.length, 1)
      assert_eq(Xml.parse(source, {"strip_blank": true}).root.nodes.length, 1)
    end

    test("Xml.well_formed? never raises") do
      assert(Xml.well_formed?("<a/>"))
      assert_not(Xml.well_formed?("<a>"))
      assert_not(Xml.well_formed?(nil))
    end

    test("Xml.escape escapes the five characters") do
      assert_eq(Xml.escape("a < b & \"c\" 'd'"), "a &lt; b &amp; &quot;c&quot; &apos;d&apos;")
    end
  end

  describe("identity and naming") do
    test("element names, prefixes and namespaces") do
      doc = Xml.parse_file(FEED)
      creator = doc.root.children[0].children[2].children[1]
      assert_eq(creator.name, "dc:creator")
      assert_eq(creator.local_name, "creator")
      assert_eq(creator.prefix, "dc")
      assert_eq(creator.namespace, "http://purl.org/dc/elements/1.1/")
      assert_eq(creator.namespaces["dc"], "http://purl.org/dc/elements/1.1/")
      assert_eq(doc.root.namespace_declarations, {"dc": "http://purl.org/dc/elements/1.1/"})
      assert_eq(creator.namespace_declarations, {})
      assert_null(doc.root.prefix)
    end

    test("kinds and predicates") do
      channel = Xml.parse_file(FEED).root.children[0]
      kinds = channel.nodes.map { |n| n.kind }.uniq
      assert(kinds.includes?("element"))
      assert(kinds.includes?("comment"))
      assert(kinds.includes?("text"))
      cdata = channel.children[2].children[2].nodes[0]
      assert(cdata.cdata?)
      assert_eq(cdata.text, "<p>Hello</p>")
      assert_null(cdata.name)
    end

    test("path") do
      items = Xml.parse(SMALL).root.children[0].children
      assert_eq(items[2].path, "/rss/channel/item[2]")
    end

    test("declaration") do
      assert_eq(Xml.parse_file(FEED).declaration["version"], "1.0")
      assert_null(Xml.parse("<a/>").declaration)
    end
  end

  describe("navigation") do
    test("children are elements, nodes are everything") do
      channel = Xml.parse_file(FEED).root.children[0]
      assert_eq(channel.children.map { |c| c.name }, ["title", "link", "item", "item"])
      assert(channel.nodes.length > channel.children.length)
    end

    test("parent, document, siblings, ancestors, descendants") do
      doc = Xml.parse(SMALL)
      channel = doc.root.children[0]
      first = channel.children[1]
      assert(first.parent == channel)
      assert_null(doc.root.parent)
      assert(first.document == doc)
      assert_eq(first.next_sibling["id"], "2")
      assert(first.next_sibling.previous_sibling == first)
      assert_null(channel.children[2].next_sibling)
      assert_eq(first.ancestors.map { |a| a.name }, ["channel", "rss"])
      assert_eq(doc.root.descendants.length, 5)
      assert_eq(first.next_node.name, "item")
    end

    test("every array is fresh") do
      root = Xml.parse(SMALL).root
      kids = root.children
      kids.push(1)
      assert_eq(root.children.length, 1)
    end
  end

  describe("text and attributes") do
    test("text is the decoded string-value, own_text the direct runs") do
      doc = Xml.parse("<a>x <b>y</b> &amp; z</a>")
      assert_eq(doc.root.text, "x y & z")
      assert_eq(doc.root.own_text, "x  & z")
      assert_eq(doc.root.inner_xml, "x <b>y</b> &amp; z")
    end

    test("attributes by index, get, attribute and attributes") do
      root = Xml.parse("<a id=\"1\" xmlns:x=\"urn:x\" x:k=\"v\"/>").root
      assert_eq(root["id"], "1")
      assert_eq(root.get("id"), "1")
      assert_null(root["nope"])
      assert_null(root["__xml"])
      assert_eq(root.attribute("k", "urn:x"), "v")
      assert_null(root.attribute("k"))
      assert_eq(root.attributes, {"id": "1", "x:k": "v"})
      assert(root.has_attribute?("id"))
      assert_not(root.has_attribute?("nope"))
    end

    test("a non-element has no attributes") do
      text = Xml.parse("<a>t</a>").root.nodes[0]
      assert_null(text["id"])
      assert_eq(text.attributes, {})
    end
  end

  describe("serialisation") do
    test("to_xml is lossless and keeps self-closing bits") do
      source = "<a x=\"1\"><b/><c></c><!--k--><?p d?></a>"
      assert_eq(Xml.parse(source).root.to_xml, source)
      assert_eq(Xml.parse(source).root.to_s, source)
    end

    test("to_xml options") do
      doc = Xml.parse("<a><b>t</b></a>")
      assert_eq(doc.to_xml(indent: 2), "<a>\n  <b>t</b>\n</a>")
      assert_eq(doc.root.to_xml(declaration: true), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a><b>t</b></a>")
      assert_eq(Xml.parse("<a/>").to_xml(expand_empty: true), "<a></a>")
      assert_raises("unknown option 'pretty'") do
        doc.to_xml(pretty: true)
      end
    end

    test("an element's to_xml declares the namespaces it uses") do
      doc = Xml.parse("<r xmlns:p=\"urn:p\"><p:a/></r>")
      assert_eq(doc.root.children[0].to_xml, "<p:a xmlns:p=\"urn:p\"/>")
    end
  end

  describe("to_h") do
    test("the lossy view") do
      h = Xml.parse("<item id=\"1\"><title>A &amp; B</title><tag>x</tag><tag>y</tag></item>").to_h
      assert_eq(h, {"item": {"@id": "1", "title": "A & B", "tag": ["x", "y"]}})
    end

    test("empty and mixed elements") do
      assert_eq(Xml.parse("<a/>").to_h, {"a": nil})
      assert_eq(Xml.parse("<a x=\"1\">t<b/></a>").to_h, {"a": {"@x": "1", "b": nil, "_text": "t"}})
    end
  end

  describe("display, JSON and equality") do
    test("print, interpolation and inspect show the compact XML") do
      item = Xml.parse(SMALL).root.children[0].children[2]
      assert_eq("#{item}", "<item id=\"2\"/>")
      assert_eq(item.inspect, "<item id=\"2\"/>")
      assert_eq(item.class, "XmlNode")
    end

    test("JSON gives the XML string") do
      item = Xml.parse(SMALL).root.children[0].children[2]
      assert_eq(json_stringify({"n": item}), "{\"n\":\"<item id=\\\"2\\\"/>\"}")
    end

    test("== is identity") do
      doc = Xml.parse(SMALL)
      items = doc.root.children[0].children
      assert(items[1] == doc.root.children[0].children[1])
      assert(items[1] != items[2])
      assert(items[1].parent == items[2].parent)
      assert_eq([items[1], items[1].next_sibling.previous_sibling].uniq.length, 1)
      assert([items[1]].includes?(doc.root.children[0].children[1]))
      assert(Xml.parse(SMALL) != Xml.parse(SMALL))
      assert_eq(Xml.parse(SMALL).to_xml, Xml.parse(SMALL).to_xml)
    end
  end

  describe("handles") do
    test("XmlNode has no constructor") do
      assert_raises("XmlNode has no constructor") do
        XmlNode.new
      end
      assert_raises("XmlNode has no constructor") do
        XmlNode.new()
      end
      assert_raises("XmlNode has no constructor") do
        XmlNode.new("x")
      end
      assert_raises("XmlNode has no constructor") do
        XmlNode.new(name: "x")
      end
    end

    test("XmlDocument has no constructor") do
      assert_raises("XmlDocument has no constructor") do
        XmlDocument.new
      end
      assert_raises("XmlDocument has no constructor") do
        XmlDocument.new()
      end
    end

    test("a dot write other than text and name is refused") do
      root = Xml.parse("<a/>").root
      assert_raises("XmlNode has no writable property 'foo'") do
        root.foo = 1
      end
    end
  end
end
