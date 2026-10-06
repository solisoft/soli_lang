# SOAP: building envelopes (wrap, xml_escape, to_xml) and reading responses
# (parse). SOAP.call needs a server and is not exercised here.

const SOAP11_NS = "http://schemas.xmlsoap.org/soap/envelope/"

def envelope_for(namespace, body)
  "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<soap:Envelope xmlns:soap=\"#{namespace}\">\n" +
    "  <soap:Body>\n    #{body}\n  </soap:Body>\n</soap:Envelope>"
end

describe("SOAP") do
  describe("SOAP.wrap") do
    test("wraps a body in a SOAP 1.1 envelope") do
      assert_eq(SOAP.wrap("<Test>Hello</Test>"), envelope_for(SOAP11_NS, "<Test>Hello</Test>"))
    end

    test("takes the envelope namespace as a second argument") do
      assert_eq(SOAP.wrap("<Test/>", "http://example.com/ns"), envelope_for("http://example.com/ns", "<Test/>"))
    end

    test("passes an XML body through verbatim by default") do
      assert_contains(SOAP.wrap("<a>&amp;</a>"), "    <a>&amp;</a>\n")
    end

    test("escape: true turns an untrusted body into text") do
      envelope = SOAP.wrap("<b>&</b>", nil, {"escape": true})
      assert_eq(envelope, envelope_for(SOAP11_NS, "&lt;b&gt;&amp;&lt;/b&gt;"))
    end

    test("refuses a body that is not a string") do
      assert_raises("SOAP.wrap() expects string body, got int") do
        SOAP.wrap(42)
      end
    end
  end

  describe("SOAP.xml_escape") do
    test("escapes all five XML special characters") do
      assert_eq(SOAP.xml_escape("<>&'\""), "&lt;&gt;&amp;&apos;&quot;")
    end

    test("escapes characters inside ordinary text") do
      assert_eq(SOAP.xml_escape("a < b & c"), "a &lt; b &amp; c")
    end

    test("leaves plain text and the empty string alone") do
      assert_eq(SOAP.xml_escape("plain text"), "plain text")
      assert_eq(SOAP.xml_escape(""), "")
    end
  end

  describe("SOAP.to_xml") do
    test("converts a hash to elements under a root") do
      assert_eq(SOAP.to_xml({"name": "test", "value": 42}), "<root><name>test</name>\n<value>42</value></root>")
    end

    test("nests hashes, numbers arrays, empties nil and escapes text") do
      xml = SOAP.to_xml({"a": {"b": "x<y"}, "list": [1, 2], "missing": nil, "flag": true})
      expected = "<root><a><b>x&lt;y</b></a>\n<list_0>1</list_0>\n<list_1>2</list_1>\n" +
        "<missing />\n<flag>true</flag></root>"
      assert_eq(xml, expected)
    end

    test("refuses something that is not a hash") do
      assert_raises("SOAP.to_xml() expects hash, got string") do
        SOAP.to_xml("text")
      end
    end

    test("takes the root element name as a second argument") do
      pending("bug: SOAP.to_xml is registered with arity 1, so its root_element argument raises")
      assert_eq(SOAP.to_xml({"a": 1}, "request"), "<request><a>1</a></request>")
    end
  end

  describe("SOAP.parse") do
    test("reads a SOAP response into nested hashes") do
      xml = "<soap:Envelope xmlns:soap=\"#{SOAP11_NS}\"><soap:Body><GetPriceResponse>" +
        "<Price>1.90</Price><Name>Tea</Name></GetPriceResponse></soap:Body></soap:Envelope>"
      response = SOAP.parse(xml)["soap:Envelope"]["soap:Body"]["GetPriceResponse"]
      assert_eq(response, {"Price": "1.90", "Name": "Tea"})
    end

    test("collects repeated elements into an array") do
      assert_eq(SOAP.parse("<root><item>1</item><item>2</item></root>"), {"root": {"item": ["1", "2"]}})
    end

    test("round-trips what wrap built") do
      parsed = SOAP.parse(SOAP.wrap("<Ping>ok</Ping>"))
      assert_eq(parsed["soap:Envelope"]["soap:Body"]["Ping"], "ok")
    end

    test("refuses a DOCTYPE (XXE guard)") do
      assert_raises("DOCTYPE declarations are not allowed in SOAP payloads") do
        SOAP.parse("<!DOCTYPE x [<!ENTITY a \"b\">]><root/>")
      end
    end

    test("refuses malformed XML") do
      assert_raises("expected `</unclosed>`, but `</root>` was found") do
        SOAP.parse("<root><unclosed></root>")
      end
    end
  end
end
