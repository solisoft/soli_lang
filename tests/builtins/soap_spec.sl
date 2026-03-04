// SOAP Class Tests
// Tests SOAP.call, SOAP.wrap, SOAP.parse, SOAP.xml_escape, SOAP.to_xml

describe("SOAP", fn() {
    test("SOAP.wrap creates SOAP envelope", fn() {
        let body = "<Test>Hello</Test>";
        let envelope = SOAP.wrap(body);
        print("SOAP.wrap result:", envelope);
        assert(envelope.contains("soap:Envelope"));
        assert(envelope.contains(body));
    });

    test("SOAP.wrap with namespace", fn() {
        let body = "<Test>Hello</Test>";
        let envelope = SOAP.wrap(body, "http://example.com/ns");
        print("SOAP.wrap with ns:", envelope);
        assert(envelope.contains("http://example.com/ns"));
    });

    test("SOAP.xml_escape escapes XML special chars", fn() {
        let escaped = SOAP.xml_escape("<>&'\"");
        print("SOAP.xml_escape result:", escaped);
        assert(escaped.contains("&lt;"));
        assert(escaped.contains("&gt;"));
        assert(escaped.contains("&amp;"));
    });

    test("SOAP.to_xml converts hash to XML", fn() {
        let data = {"name" => "test", "value" => 42};
        let xml = SOAP.to_xml(data);
        print("SOAP.to_xml result:", xml);
        assert(xml.contains("<name>test</name>"));
        assert(xml.contains("<value>42</value>"));
    });
});
