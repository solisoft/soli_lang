# The c14n trio (Xml.c14n_exclusive, get_element_by_id, get_elements_by_tag)
# on edge inputs, frozen before parse_document moved onto the shared XML core
# (src/xml). Every row must stay byte-identical except a row tagged
# `# unavoidable:` or `# limit:`, which shows the old output above it.

def capture(f)
  result = nil
  try
    result = f()
  catch err
    result = "ERR " + Regex.replace_all(" at \\d+:\\d+$", "#{err}", "")
  end
  result
end

# `depth` nested <n> elements around `inner`.
def deep(depth, inner)
  "<n>" * depth + inner + "</n>" * depth
end

def c14n_json(xml, opts = nil)
  capture(fn() { json_stringify(Xml.c14n_exclusive(xml, opts)) })
end

def by_id_json(xml, id)
  capture(fn() { json_stringify(Xml.get_element_by_id(xml, id)) })
end

def by_tag_json(xml, tag)
  capture(fn() { json_stringify(Xml.get_elements_by_tag(xml, tag)) })
end

describe("c14n trio compatibility") do
  describe("Xml.c14n_exclusive") do
    test("CRLF in text") do
      assert_eq(c14n_json("<a>l1\r\nl2\rl3</a>"), "\"<a>l1\\nl2\\nl3</a>\"")
    end
    test("CR charref in text") do
      assert_eq(c14n_json("<a>x&#13;y&#xD;</a>"), "\"<a>x&#xD;y&#xD;</a>\"")
    end
    test("CRLF in attribute") do
      assert_eq(c14n_json("<a x=\"1\r\n2\"/>"), "\"<a x=\\\"1 2\\\"></a>\"")
    end
    test("tab and LF in attribute") do
      assert_eq(c14n_json("<a x=\"1\t2\n3\"/>"), "\"<a x=\\\"1 2 3\\\"></a>\"")
    end
    test("charrefs in attribute") do
      assert_eq(c14n_json("<a x=\"&#9;&#10;&#13;&lt;&amp;&quot;\"/>"), "\"<a x=\\\"&#x9;&#xA;&#xD;&lt;&amp;&quot;\\\"></a>\"")
    end
    test("entities in text") do
      assert_eq(c14n_json("<a>&lt;&gt;&amp;&quot;&apos;</a>"), "\"<a>&lt;&gt;&amp;\\\"'</a>\"")
    end
    test("control charref in text") do
      assert_eq(c14n_json("<a>x&#1;y</a>"), "\"<a>x\\u0001y</a>\"")
    end
    test("control charref in attribute") do
      assert_eq(c14n_json("<a x=\"&#1;\"/>"), "\"<a x=\\\"\\u0001\\\"></a>\"")
    end
    test("NUL charref") do
      # message: refused as before, worded by the XML core
      # was: ERR Xml.c14n_exclusive(): XML character reference error: invalid character reference: 0x0 character is not permitted in XML
      assert_eq(c14n_json("<a>&#0;</a>"), "ERR Xml.c14n_exclusive(): XML parsing error: character reference to U+0000, which XML forbids")
    end
    test("FFFE charref") do
      assert_eq(c14n_json("<a>&#xFFFE;</a>"), "\"<a>\u{fffe}</a>\"")
    end
    test("unknown entity") do
      assert_eq(c14n_json("<a>&foo;</a>"), "ERR Xml.c14n_exclusive(): general entity reference `&foo;` is not allowed (XXE / entity-expansion defense)")
    end
    test("unknown entity in attribute") do
      # message: refused as before, worded as for an entity in text
      # was: ERR Xml.c14n_exclusive(): XML attribute value error: at 1..4: unrecognized entity `foo`
      assert_eq(c14n_json("<a x=\"&foo;\"/>"), "ERR Xml.c14n_exclusive(): general entity reference `&foo;` is not allowed (XXE / entity-expansion defense)")
    end
    test("PI inside root") do
      assert_eq(c14n_json("<a><?pi  data  here?><?empty?></a>"), "\"<a><?pi   data  here?><?empty?></a>\"")
    end
    test("PI outside root") do
      assert_eq(c14n_json("<?pi out?><a>x</a><?pi after?>"), "\"<a>x</a>\"")
    end
    test("comments") do
      assert_eq(c14n_json("<!--c--><a>x<!-- y -->z</a><!--d-->"), "\"<a>xz</a>\"")
    end
    test("cdata") do
      assert_eq(c14n_json("<a><![CDATA[<x>&]]></a>"), "\"<a>&lt;x&gt;&amp;</a>\"")
    end
    test("text outside root") do
      assert_eq(c14n_json("  <a>x</a>  "), "\"<a>x</a>\"")
    end
    test("second root ignored") do
      assert_eq(c14n_json("<a>1</a><b>2</b>"), "\"<a>1</a>\"")
    end
    test("second root with bad entity") do
      assert_eq(c14n_json("<a>1</a><b>&foo;</b>"), "ERR Xml.c14n_exclusive(): general entity reference `&foo;` is not allowed (XXE / entity-expansion defense)")
    end
    test("unclosed second root") do
      assert_eq(c14n_json("<a>1</a><b>"), "\"<a>1</a>\"")
    end
    test("unclosed root") do
      assert_eq(c14n_json("<a><b>1</b>"), "ERR Xml.c14n_exclusive(): no document element found")
    end
    test("empty input") do
      assert_eq(c14n_json(""), "ERR Xml.c14n_exclusive(): no document element found")
    end
    test("text only") do
      assert_eq(c14n_json("hello"), "ERR Xml.c14n_exclusive(): no document element found")
    end
    test("mismatched end") do
      assert_eq(c14n_json("<a><b></a>"), "ERR Xml.c14n_exclusive(): XML parsing error: ill-formed document: expected `</b>`, but `</a>` was found")
    end
    test("doctype") do
      assert_eq(c14n_json("<!DOCTYPE a><a/>"), "ERR Xml.c14n_exclusive(): DOCTYPE declarations are not allowed (XXE / entity-expansion defense)")
    end
    test("declaration with bare encoding") do
      assert_eq(c14n_json("<?xml version=\"1.0\" encoding?><a/>"), "\"<a></a>\"")
    end
    test("xmlns empty undeclares") do
      assert_eq(c14n_json("<r xmlns=\"u\"><c xmlns=\"\"><d/></c></r>"), "\"<r xmlns=\\\"u\\\"><c xmlns=\\\"\\\"><d></d></c></r>\"")
    end
    test("unbound prefix element") do
      assert_eq(c14n_json("<p:a><p:b/></p:a>"), "\"<p:a><p:b></p:b></p:a>\"")
    end
    test("unbound prefix attribute beside plain") do
      assert_eq(c14n_json("<a x=\"1\" p:x=\"2\"/>"), "\"<a x=\\\"1\\\" p:x=\\\"2\\\"></a>\"")
    end
    test("two prefixes same uri same local") do
      assert_eq(c14n_json("<a xmlns:p=\"u\" xmlns:q=\"u\" p:x=\"1\" q:x=\"2\"/>"), "\"<a xmlns:p=\\\"u\\\" xmlns:q=\\\"u\\\" p:x=\\\"1\\\" q:x=\\\"2\\\"></a>\"")
    end
    test("duplicate attribute") do
      # message: refused as before, worded by the XML core
      # was: ERR Xml.c14n_exclusive(): XML attribute error: position 8: duplicated attribute, previous declaration at position 2
      assert_eq(c14n_json("<a x=\"1\" x=\"2\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: duplicate attribute 'x'")
    end
    test("duplicate declaration") do
      # message: refused as before, worded by the XML core
      # was: ERR Xml.c14n_exclusive(): XML attribute error: position 14: duplicated attribute, previous declaration at position 2
      assert_eq(c14n_json("<a xmlns:p=\"u\" xmlns:p=\"v\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: duplicate attribute 'xmlns:p'")
    end
    test("xml prefix rebound") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: "<a></a>"
      assert_eq(c14n_json("<a xmlns:xml=\"other\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: the namespace prefix 'xml' cannot be bound to '\"other\"'")
    end
    test("xmlns prefix declared") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: "<a></a>"
      assert_eq(c14n_json("<a xmlns:xmlns=\"u\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: the namespace prefix 'xmlns' cannot be bound to '\"u\"'")
    end
    test("prefix bound to xml uri") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: "<a></a>"
      assert_eq(c14n_json("<a xmlns:p=\"http://www.w3.org/XML/1998/namespace\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: the namespace prefix '\"p\"' cannot be bound to 'http://www.w3.org/XML/1998/namespace'")
    end
    test("xml lang attribute") do
      assert_eq(c14n_json("<a xml:lang=\"en\" b=\"1\"/>"), "\"<a b=\\\"1\\\" xml:lang=\\\"en\\\"></a>\"")
    end
    test("Id spellings") do
      assert_eq(c14n_json("<a ID=\"1\"><b Id=\"2\"/><c wsu:Id=\"3\" xmlns:wsu=\"w\"/></a>"), "\"<a ID=\\\"1\\\"><b Id=\\\"2\\\"></b><c xmlns:wsu=\\\"w\\\" wsu:Id=\\\"3\\\"></c></a>\"")
    end
    test("id option") do
      assert_eq(c14n_json("<r xmlns:a=\"u\"><a:x ID=\"t\"><a:y>1</a:y></a:x></r>", {"id": "t"}), "\"<a:x xmlns:a=\\\"u\\\" ID=\\\"t\\\"><a:y>1</a:y></a:x>\"")
    end
    test("id option wsu") do
      assert_eq(c14n_json("<r xmlns:wsu=\"w\"><x wsu:Id=\"t\">1</x></r>", {"id": "t"}), "\"<x xmlns:wsu=\\\"w\\\" wsu:Id=\\\"t\\\">1</x>\"")
    end
    test("first id attribute wins") do
      assert_eq(c14n_json("<r><x id=\"a\" ID=\"b\">1</x></r>", {"id": "b"}), "ERR Xml.c14n_exclusive(): no element with Id 'b' found")
    end
    test("inclusive prefixes") do
      assert_eq(c14n_json("<r xmlns:a=\"u\" xmlns:b=\"v\"><a:x/></r>", "b"), "\"<r xmlns:b=\\\"v\\\"><a:x xmlns:a=\\\"u\\\"></a:x></r>\"")
    end
    test("inclusive default") do
      assert_eq(c14n_json("<r xmlns=\"u\"><x xmlns=\"\"/></r>", "#default"), "\"<r xmlns=\\\"u\\\"><x xmlns=\\\"\\\"></x></r>\"")
    end
    test("enveloped") do
      assert_eq(c14n_json("<r ID=\"d\"><x>1</x><ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"><ds:S/></ds:Signature></r>", {"id": "d", "enveloped_signature": true}), "\"<r ID=\\\"d\\\"><x>1</x></r>\"")
    end
    test("whitespace kept inside") do
      assert_eq(c14n_json("<a>\n  <b> x </b>\n</a>"), "\"<a>\\n  <b> x </b>\\n</a>\"")
    end
    test("BOM") do
      assert_eq(c14n_json("\u{feff}<a>x</a>"), "\"<a>x</a>\"")
    end
    test("single-quoted attribute") do
      assert_eq(c14n_json("<a b='\"x\"'/>"), "\"<a b=\\\"&quot;x&quot;\\\"></a>\"")
    end
    test("attribute sorting by namespace") do
      assert_eq(c14n_json("<a xmlns:z=\"a\" xmlns:b=\"z\" b:k=\"1\" z:k=\"2\" k=\"0\"/>"), "\"<a xmlns:b=\\\"z\\\" xmlns:z=\\\"a\\\" k=\\\"0\\\" z:k=\\\"2\\\" b:k=\\\"1\\\"></a>\"")
    end
    test("attribute without value") do
      # message: refused as before, worded by the XML core
      # was: ERR Xml.c14n_exclusive(): XML attribute error: position 3: attribute key must be directly followed by `=` or space
      assert_eq(c14n_json("<a b/>"), "ERR Xml.c14n_exclusive(): XML parsing error: error while parsing attribute: position 3: attribute key must be directly followed by `=` or space")
    end
    test("ampersand in attribute") do
      # message: refused as before, worded by the XML core
      # was: ERR Xml.c14n_exclusive(): XML attribute value error: Error while escaping character at range 2..4: Cannot find ';' after '&'
      assert_eq(c14n_json("<a x=\"AT&T\"/>"), "ERR Xml.c14n_exclusive(): XML parsing error: entity or character reference not closed in an attribute value")
    end
    test("lt in attribute") do
      assert_eq(c14n_json("<a x=\"<\"/>"), "\"<a x=\\\"&lt;\\\"></a>\"")
    end
    test("literal control char") do
      assert_eq(c14n_json("<a>x\u{1}y</a>"), "\"<a>x\\u0001y</a>\"")
    end
    test("comment with double hyphen") do
      assert_eq(c14n_json("<a><!-- a -- b -->x</a>"), "\"<a>x</a>\"")
    end
    test("declaration after root") do
      assert_eq(c14n_json("<a/><?xml version=\"1.0\"?>"), "\"<a></a>\"")
    end
    test("300 declarations on one element") do
      assert_eq(c14n_json("<a " + (0..300).map { |i| "xmlns:p#{i}=\"u#{i}\"" }.join(" ") + "/>").length.to_s, "9")
    end
    test("depth 256") do
      assert_eq(c14n_json(deep(256, "")).length.to_s, "1794")
    end
    test("depth 257") do
      assert_eq(c14n_json(deep(257, "")), "ERR Xml.c14n_exclusive(): nesting depth exceeded 256 levels")
    end
    test("depth 256 plus self-closing leaf") do
      assert_eq(c14n_json(deep(256, "<leaf/>")).length.to_s, "1807")
    end
    test("2000 attributes") do
      # limit: at most 1024 attributes per element (SOLI_MAX_XML_ATTRS)
      # was: 27789
      assert_eq(c14n_json("<a " + (0..2000).map { |i| "x#{i}=\"#{i}\"" }.join(" ") + "/>").length.to_s, "106")
    end
    test("name of 5000 bytes") do
      # limit: names are at most 4096 bytes (MAX_NAME_LEN)
      # was: 10007
      assert_eq(c14n_json("<" + "n" * 5000 + "/>").length.to_s, "72")
    end
  end

  describe("Xml.get_element_by_id") do
    test("inherited namespaces injected") do
      assert_eq(by_id_json("<r xmlns=\"d\" xmlns:a=\"u\" xmlns:unused=\"z\"><a:x ID=\"t\"><y/></a:x></r>", "t"), "\"<a:x xmlns=\\\"d\\\" xmlns:a=\\\"u\\\" xmlns:unused=\\\"z\\\" ID=\\\"t\\\"><y xmlns=\\\"d\\\" xmlns:a=\\\"u\\\" xmlns:unused=\\\"z\\\"></y></a:x>\"")
    end
    test("local redeclaration wins") do
      assert_eq(by_id_json("<r xmlns:a=\"u\"><a:x xmlns:a=\"v\" ID=\"t\"/></r>", "t"), "\"<a:x xmlns:a=\\\"v\\\" ID=\\\"t\\\"></a:x>\"")
    end
    test("undeclared default not injected") do
      assert_eq(by_id_json("<r xmlns=\"d\"><x xmlns=\"\" ID=\"t\"/></r>", "t"), "\"<x xmlns=\\\"\\\" ID=\\\"t\\\"></x>\"")
    end
    test("text and PI kept, comment dropped") do
      assert_eq(by_id_json("<r><x ID=\"t\">a &amp; b<?p d?><!--c--><![CDATA[<]]></x></r>", "t"), "\"<x ID=\\\"t\\\">a &amp; b<?p  d?>&lt;</x>\"")
    end
    test("attribute escaping") do
      assert_eq(by_id_json("<r><x ID=\"t\" v=\"a&#9;b&quot;c\"/></r>", "t"), "\"<x ID=\\\"t\\\" v=\\\"a&#x9;b&quot;c\\\"></x>\"")
    end
    test("missing id") do
      assert_eq(by_id_json("<r/>", "nope"), "ERR Xml.get_element_by_id(): no element with Id 'nope' found")
    end
  end

  describe("Xml.get_elements_by_tag") do
    test("nested matches both returned") do
      assert_eq(by_tag_json("<r><Item>1<Item>2</Item></Item><g><Item>3</Item></g></r>", "Item"), "[\"<Item>1<Item>2</Item></Item>\",\"<Item>2</Item>\",\"<Item>3</Item>\"]")
    end
    test("prefixed match by local name") do
      assert_eq(by_tag_json("<r xmlns:a=\"u\" xmlns:b=\"v\"><a:Item>1</a:Item><b:Item>2</b:Item></r>", "Item"), "[\"<a:Item xmlns:a=\\\"u\\\" xmlns:b=\\\"v\\\">1</a:Item>\",\"<b:Item xmlns:a=\\\"u\\\" xmlns:b=\\\"v\\\">2</b:Item>\"]")
    end
    test("inherited namespaces per match") do
      assert_eq(by_tag_json("<r xmlns:a=\"u\"><s xmlns:b=\"v\"><a:Item><b:x/></a:Item></s></r>", "Item"), "[\"<a:Item xmlns:a=\\\"u\\\" xmlns:b=\\\"v\\\"><b:x xmlns:a=\\\"u\\\" xmlns:b=\\\"v\\\"></b:x></a:Item>\"]")
    end
    test("root matches") do
      assert_eq(by_tag_json("<Item xmlns=\"d\"><Item/></Item>", "Item"), "[\"<Item xmlns=\\\"d\\\"><Item xmlns=\\\"d\\\"></Item></Item>\",\"<Item xmlns=\\\"d\\\"></Item>\"]")
    end
    test("no match") do
      assert_eq(by_tag_json("<r/>", "Item"), "[]")
    end
  end
end
