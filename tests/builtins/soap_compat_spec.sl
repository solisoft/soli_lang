# SOAP.parse compatibility table: the output of today's SOAP.parse (the
# quick-xml loop in soap.rs as of 2.29.5), frozen before SOAP.parse moved onto
# the shared XML core (src/xml). Every row must stay byte-identical, except
# rows tagged `# fix:` (a change the user approved: the six SOAP.parse bugs and
# CRLF normalisation), `# unavoidable:` (a refusal of the quick-xml NsReader
# the core is built on) or `# limit:` (a hardening limit of the core). Those
# rows show the old output in a comment above the new expectation.
#
# Values are compared as json_stringify text, so key order counts. Errors are
# compared without the " at L:C" the engine appends.

def soap_json(xml)
  result = nil
  try
    result = json_stringify(SOAP.parse(xml))
  catch err
    result = "ERR " + Regex.replace_all(" at \\d+:\\d+$", "#{err}", "")
  end
  result
end

# Large outputs are compared by length and their first 80 characters.
def soap_summary(xml)
  out = soap_json(xml)
  "#{out.length}:#{out.substring(0, 80)}"
end

describe("SOAP.parse compatibility") do
  describe("entities and character references") do
    test("entity between words") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"a":"x&y"}
      assert_eq(soap_json("<a>x &amp; y</a>"), "{\"a\":\"x & y\"}")
    end
    test("entity in child") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"a":{"b":"Tom&Jerry"}}
      assert_eq(soap_json("<a><b>Tom &amp; Jerry</b></a>"), "{\"a\":{\"b\":\"Tom & Jerry\"}}")
    end
    test("entities only") do
      assert_eq(soap_json("<a>&lt;tag&gt;</a>"), "{\"a\":\"<tag>\"}")
    end
    test("quot and apos") do
      assert_eq(soap_json("<a>&quot;&apos;</a>"), "{\"a\":\"\\\"'\"}")
    end
    test("decimal charref mid-word") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"a":"caféok"}
      assert_eq(soap_json("<a>caf&#233; ok</a>"), "{\"a\":\"caf\u{e9} ok\"}")
    end
    test("hex and decimal charrefs") do
      assert_eq(soap_json("<a>&#x41;&#66;</a>"), "{\"a\":\"AB\"}")
    end
    test("entity at start and end") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"a":"&x&"}
      assert_eq(soap_json("<a>&amp; x &amp;</a>"), "{\"a\":\"& x &\"}")
    end
    test("unknown entity between words") do
      # fix: whitespace around entities is kept (bug 1); the unknown entity is still dropped
      # was: {"a":"xy"}
      assert_eq(soap_json("<a>x &foo; y</a>"), "{\"a\":\"x  y\"}")
    end
    test("unknown entity alone") do
      assert_eq(soap_json("<a>&foo;</a>"), "{\"a\":null}")
    end
    test("charref NUL") do
      assert_eq(soap_json("<a>&#0;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref control 1") do
      assert_eq(soap_json("<a>&#1;v</a>"), "{\"a\":\"\\u0001v\"}")
    end
    test("charref surrogate") do
      assert_eq(soap_json("<a>&#xD800;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref past unicode") do
      assert_eq(soap_json("<a>&#x110000;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref uppercase X") do
      assert_eq(soap_json("<a>&#X41;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref FFFE") do
      assert_eq(soap_json("<a>&#xFFFE;v</a>"), "{\"a\":\"\u{fffe}v\"}")
    end
    test("charref empty decimal") do
      assert_eq(soap_json("<a>&#;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref empty hex") do
      assert_eq(soap_json("<a>&#x;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref negative") do
      assert_eq(soap_json("<a>&#-1;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref plus sign") do
      assert_eq(soap_json("<a>&#+65;v</a>"), "{\"a\":\"v\"}")
    end
    test("charref space only") do
      assert_eq(soap_json("<a>&#32;</a>"), "{\"a\":null}")
    end
    test("charref CR inside word") do
      assert_eq(soap_json("<a>x&#13;y</a>"), "{\"a\":\"x\\ry\"}")
    end
    test("charref tab inside word") do
      assert_eq(soap_json("<a>x&#9;y</a>"), "{\"a\":\"x\\ty\"}")
    end
    test("dangling ampersand") do
      assert_eq(soap_json("<a>a &amp b</a>"), "ERR XML parsing error: ill-formed document: entity or character reference not closed: `;` not found before end of input")
    end
    test("bare ampersand") do
      assert_eq(soap_json("<a>a & b</a>"), "ERR XML parsing error: ill-formed document: entity or character reference not closed: `;` not found before end of input")
    end
    test("entity before root") do
      assert_eq(soap_json("&amp;<a>x</a>"), "{\"a\":\"&x\"}")
    end
    test("unknown entity before root") do
      assert_eq(soap_json("&foo;<a>x</a>"), "{\"a\":\"x\"}")
    end
    test("NUL charref before root") do
      assert_eq(soap_json("&#0;<a>x</a>"), "{\"a\":\"x\"}")
    end
    test("entity in nested text") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"r":{"a":"1<2","b":"3>2"}}
      assert_eq(soap_json("<r><a>1 &lt; 2</a><b>3 &gt; 2</b></r>"), "{\"r\":{\"a\":\"1 < 2\",\"b\":\"3 > 2\"}}")
    end
  end

  describe("CDATA") do
    test("cdata only") do
      # fix: CDATA is kept (bug 2)
      # was: {"a":null}
      assert_eq(soap_json("<a><![CDATA[ <b>raw</b> ]]></a>"), "{\"a\":\"<b>raw</b>\"}")
    end
    test("cdata between text") do
      # fix: CDATA is kept (bug 2)
      # was: {"a":"xz"}
      assert_eq(soap_json("<a>x<![CDATA[y]]>z</a>"), "{\"a\":\"xyz\"}")
    end
    test("cdata spaced") do
      # fix: CDATA is kept (bug 2)
      # was: {"a":"xz"}
      assert_eq(soap_json("<a>x <![CDATA[ y ]]> z</a>"), "{\"a\":\"x  y  z\"}")
    end
    test("empty cdata") do
      assert_eq(soap_json("<a><![CDATA[]]></a>"), "{\"a\":null}")
    end
    test("cdata before child") do
      # fix: CDATA is kept (bug 2)
      # was: {"r":{"c":"1"}}
      assert_eq(soap_json("<r><![CDATA[x]]><c>1</c></r>"), "{\"r\":{\"_text\":\"x\",\"c\":\"1\"}}")
    end
    test("cdata before root") do
      assert_eq(soap_json("<![CDATA[x]]><a>y</a>"), "{\"a\":\"y\"}")
    end
    test("cdata with entity text") do
      # fix: CDATA is kept (bug 2), literally
      # was: {"a":null}
      assert_eq(soap_json("<a><![CDATA[&amp;]]></a>"), "{\"a\":\"&amp;\"}")
    end
  end

  describe("self-closing elements") do
    test("self-closing root") do
      # fix: a self-closing root gives {name => nil} (bug 4)
      # was: null
      assert_eq(soap_json("<a/>"), "{\"a\":null}")
    end
    test("empty root pair") do
      assert_eq(soap_json("<a></a>"), "{\"a\":null}")
    end
    test("self-closing root with attr") do
      # fix: a self-closing root gives {name => nil} (bug 4)
      # was: null
      assert_eq(soap_json("<a x=\"1\"/>"), "{\"a\":null}")
    end
    test("self-closing root then root") do
      # fix: a self-closing root is the root (bug 4); nothing after it is read
      # was: {"b":"2"}
      assert_eq(soap_json("<a/><b>2</b>"), "{\"a\":null}")
    end
    test("self-closing root after junk") do
      # fix: a self-closing root is the root (bug 4); text before it starts its text, as for any root
      # was: null
      assert_eq(soap_json("junk<a/>"), "{\"a\":\"junk\"}")
    end
    test("self-closing child") do
      assert_eq(soap_json("<r><x/></r>"), "{\"r\":{\"x\":null}}")
    end
    test("self-closing child spaced") do
      assert_eq(soap_json("<r> <x/> </r>"), "{\"r\":{\"x\":null}}")
    end
    test("self-closing then text repeat") do
      assert_eq(soap_json("<r><x/><x>1</x></r>"), "{\"r\":{\"x\":[null,\"1\"]}}")
    end
    test("text then self-closing repeat") do
      # fix: a self-closing repeat is appended (bug 3)
      # was: {"r":{"x":null}}
      assert_eq(soap_json("<r><x>1</x><x/></r>"), "{\"r\":{\"x\":[\"1\",null]}}")
    end
    test("two self-closing repeats") do
      # fix: a self-closing repeat is appended (bug 3)
      # was: {"r":{"x":null}}
      assert_eq(soap_json("<r><x/><x/></r>"), "{\"r\":{\"x\":[null,null]}}")
    end
    test("list then self-closing") do
      # fix: a self-closing repeat is appended (bug 3)
      # was: {"r":{"x":null}}
      assert_eq(soap_json("<r><x>1</x><x>2</x><x/></r>"), "{\"r\":{\"x\":[\"1\",\"2\",null]}}")
    end
    test("self-closing in list middle") do
      # fix: a self-closing repeat is appended (bug 3)
      # was: {"r":{"x":[null,"2"]}}
      assert_eq(soap_json("<r><x>1</x><x/><x>2</x></r>"), "{\"r\":{\"x\":[\"1\",null,\"2\"]}}")
    end
    test("self-closing among others") do
      assert_eq(soap_json("<r><a>1</a><b/><c>3</c></r>"), "{\"r\":{\"a\":\"1\",\"b\":null,\"c\":\"3\"}}")
    end
    test("empty pair child") do
      assert_eq(soap_json("<r><x></x></r>"), "{\"r\":{\"x\":null}}")
    end
  end

  describe("repeats and order") do
    test("three repeats") do
      assert_eq(soap_json("<r><i>1</i><i>2</i><i>3</i></r>"), "{\"r\":{\"i\":[\"1\",\"2\",\"3\"]}}")
    end
    test("nested repeat array") do
      assert_eq(soap_json("<r><i><v>1</v></i><i><v>2</v></i><i>3</i></r>"), "{\"r\":{\"i\":[{\"v\":\"1\"},{\"v\":\"2\"},\"3\"]}}")
    end
    test("interleaved repeats keep first position") do
      assert_eq(soap_json("<r><a>1</a><b>2</b><a>3</a></r>"), "{\"r\":{\"a\":[\"1\",\"3\"],\"b\":\"2\"}}")
    end
    test("deep repeat") do
      assert_eq(soap_json("<r><g><i>1</i><i>2</i></g><g><i>3</i></g></r>"), "{\"r\":{\"g\":[{\"i\":[\"1\",\"2\"]},{\"i\":\"3\"}]}}")
    end
    test("text zero stays string") do
      assert_eq(soap_json("<a>0</a>"), "{\"a\":\"0\"}")
    end
    test("numbers stay strings") do
      assert_eq(soap_json("<r><n>42</n><f>1.5</f><b>true</b></r>"), "{\"r\":{\"n\":\"42\",\"f\":\"1.5\",\"b\":\"true\"}}")
    end
  end

  describe("namespaces and prefixes") do
    test("prefixes kept as written") do
      assert_eq(soap_json("<s:E xmlns:s=\"u\"><s:B><m:X xmlns:m=\"v\">1</m:X></s:B></s:E>"), "{\"s:E\":{\"s:B\":{\"m:X\":\"1\"}}}")
    end
    test("default namespace") do
      assert_eq(soap_json("<E xmlns=\"u\"><B>1</B></E>"), "{\"E\":{\"B\":\"1\"}}")
    end
    test("unbound prefixes") do
      assert_eq(soap_json("<p:a><q:b>1</q:b></p:a>"), "{\"p:a\":{\"q:b\":\"1\"}}")
    end
    test("same uri two prefixes") do
      assert_eq(soap_json("<r xmlns:a=\"u\" xmlns:b=\"u\"><a:x>1</a:x><b:x>2</b:x></r>"), "{\"r\":{\"a:x\":\"1\",\"b:x\":\"2\"}}")
    end
    test("empty prefixed declaration") do
      assert_eq(soap_json("<a xmlns:p=\"\">x</a>"), "{\"a\":\"x\"}")
    end
    test("undeclared default") do
      assert_eq(soap_json("<a xmlns=\"u\"><b xmlns=\"\">x</b></a>"), "{\"a\":{\"b\":\"x\"}}")
    end
    test("xml prefix attribute") do
      assert_eq(soap_json("<a xml:lang=\"en\">x</a>"), "{\"a\":\"x\"}")
    end
    test("xml prefix bound right") do
      assert_eq(soap_json("<a xmlns:xml=\"http://www.w3.org/XML/1998/namespace\">x</a>"), "{\"a\":\"x\"}")
    end
    test("xml prefix rebound") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: {"a":"x"}
      assert_eq(soap_json("<a xmlns:xml=\"other\">x</a>"), "ERR XML parsing error: the namespace prefix 'xml' cannot be bound to '\"other\"'")
    end
    test("xmlns prefix declared") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: {"a":"x"}
      assert_eq(soap_json("<a xmlns:xmlns=\"u\">x</a>"), "ERR XML parsing error: the namespace prefix 'xmlns' cannot be bound to '\"u\"'")
    end
    test("prefix bound to xml uri") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: {"a":"x"}
      assert_eq(soap_json("<a xmlns:p=\"http://www.w3.org/XML/1998/namespace\">x</a>"), "ERR XML parsing error: the namespace prefix '\"p\"' cannot be bound to 'http://www.w3.org/XML/1998/namespace'")
    end
    test("prefix bound to xmlns uri") do
      # unavoidable: quick-xml's NsReader refuses a reserved-prefix binding
      # was: {"a":"x"}
      assert_eq(soap_json("<a xmlns:p=\"http://www.w3.org/2000/xmlns/\">x</a>"), "ERR XML parsing error: the namespace prefix '\"p\"' cannot be bound to 'http://www.w3.org/2000/xmlns/'")
    end
    test("element named with xmlns prefix") do
      assert_eq(soap_json("<xmlns:a>x</xmlns:a>"), "{\"xmlns:a\":\"x\"}")
    end
    test("300 declarations on one element") do
      assert_eq(soap_json("<a " + (0..300).map { |i| "xmlns:p#{i}=\"u#{i}\"" }.join(" ") + ">x</a>"), "{\"a\":\"x\"}")
    end
    test("4097 declarations on one element") do
      # limit: at most 4096 namespace declarations live at once (MAX_NS_BINDINGS)
      # was: {"a":"x"}
      assert_eq(soap_json("<a " + (0..4097).map { |i| "xmlns:p#{i}=\"u#{i}\"" }.join(" ") + ">x</a>"), "ERR XML parsing error: start tag declares more than 4096 namespace bindings; raise the limit with NamespaceResolver::set_max_declarations_per_element")
    end
  end

  describe("mixed content") do
    test("mixed head child tail") do
      # fix: mixed-content runs are joined, then trimmed (bug 5)
      # was: {"r":{"_text":"tail","c":"1"}}
      assert_eq(soap_json("<r>head<c>1</c>tail</r>"), "{\"r\":{\"_text\":\"headtail\",\"c\":\"1\"}}")
    end
    test("mixed head mid tail") do
      # fix: mixed-content runs are joined, then trimmed (bug 5)
      # was: {"r":{"_text":"tail","c":"1","d":"2"}}
      assert_eq(soap_json("<r>head<c>1</c>mid<d>2</d>tail</r>"), "{\"r\":{\"_text\":\"headmidtail\",\"c\":\"1\",\"d\":\"2\"}}")
    end
    test("mixed around self-closing") do
      assert_eq(soap_json("<r>head<c/>tail</r>"), "{\"r\":{\"c\":null,\"_text\":\"headtail\"}}")
    end
    test("mixed spaced") do
      # fix: mixed-content runs are joined, then trimmed (bug 5)
      # was: {"r":{"_text":"tail","c":"1"}}
      assert_eq(soap_json("<r>head <c>1</c> tail</r>"), "{\"r\":{\"_text\":\"head  tail\",\"c\":\"1\"}}")
    end
    test("mixed tail only") do
      assert_eq(soap_json("<r><c>1</c>tail</r>"), "{\"r\":{\"c\":\"1\",\"_text\":\"tail\"}}")
    end
    test("mixed head only") do
      assert_eq(soap_json("<r>head<c>1</c></r>"), "{\"r\":{\"_text\":\"head\",\"c\":\"1\"}}")
    end
    test("mixed repeated text and children") do
      # fix: mixed-content runs are joined, then trimmed (bug 5)
      # was: {"r":{"_text":"a","c":["1","2"]}}
      assert_eq(soap_json("<r>a<c>1</c>a<c>2</c></r>"), "{\"r\":{\"_text\":\"aa\",\"c\":[\"1\",\"2\"]}}")
    end
    test("text before self-closing then child") do
      assert_eq(soap_json("<r>x<b/><c>2</c></r>"), "{\"r\":{\"b\":null,\"_text\":\"x\",\"c\":\"2\"}}")
    end
    test("text after child before self-closing") do
      assert_eq(soap_json("<r><a>1</a>x<b/></r>"), "{\"r\":{\"a\":\"1\",\"b\":null,\"_text\":\"x\"}}")
    end
    test("text beside child named _text") do
      assert_eq(soap_json("<r>x<_text>v</_text></r>"), "{\"r\":[\"x\",\"v\"]}")
    end
    test("child named _text alone") do
      assert_eq(soap_json("<r><_text>v</_text></r>"), "{\"r\":\"v\"}")
    end
    test("child named _text then text") do
      assert_eq(soap_json("<r><_text>v</_text>x</r>"), "{\"r\":\"x\"}")
    end
  end

  describe("comments and PIs") do
    test("comment between text") do
      assert_eq(soap_json("<a>x<!-- c -->y</a>"), "{\"a\":\"xy\"}")
    end
    test("comment between spaced text") do
      assert_eq(soap_json("<a>x <!-- c --> y</a>"), "{\"a\":\"xy\"}")
    end
    test("entity and comment") do
      # fix: whitespace around entities is kept (bug 1); the comment still cuts the text
      # was: {"a":"x&y"}
      assert_eq(soap_json("<a>x &amp; <!-- c --> y</a>"), "{\"a\":\"x &y\"}")
    end
    test("comment only") do
      assert_eq(soap_json("<a><!-- only --></a>"), "{\"a\":null}")
    end
    test("comment with double hyphen") do
      assert_eq(soap_json("<a><!-- a -- b -->x</a>"), "{\"a\":\"x\"}")
    end
    test("pi then text") do
      assert_eq(soap_json("<a><?pi data?>t</a>"), "{\"a\":\"t\"}")
    end
    test("pi between spaced text") do
      assert_eq(soap_json("<a>x <?pi d?> y</a>"), "{\"a\":\"xy\"}")
    end
    test("comments around root") do
      assert_eq(soap_json("<!-- pre --><a>1</a><!-- post -->"), "{\"a\":\"1\"}")
    end
    test("declaration and pi before root") do
      assert_eq(soap_json("<?xml version=\"1.0\"?><?style x?><a>1</a>"), "{\"a\":\"1\"}")
    end
    test("comment before child") do
      assert_eq(soap_json("<r><!-- c --><a>1</a></r>"), "{\"r\":{\"a\":\"1\"}}")
    end
    test("comment in mixed content") do
      # fix: mixed-content runs are joined, then trimmed (bug 5)
      # was: {"r":{"_text":"d","c":"1"}}
      assert_eq(soap_json("<r>a<!--c-->b<c>1</c>d</r>"), "{\"r\":{\"_text\":\"abd\",\"c\":\"1\"}}")
    end
    test("xml-named pi") do
      assert_eq(soap_json("<a><?xml-stylesheet href=\"x\"?>t</a>"), "{\"a\":\"t\"}")
    end
  end

  describe("text outside the root") do
    test("text before root") do
      assert_eq(soap_json("junk<a>x</a>"), "{\"a\":\"junkx\"}")
    end
    test("spaced text before root") do
      assert_eq(soap_json("  junk  <a> x </a>"), "{\"a\":\"junkx\"}")
    end
    test("text before root with child") do
      assert_eq(soap_json("junk<a><b>1</b></a>"), "{\"a\":{\"_text\":\"junk\",\"b\":\"1\"}}")
    end
    test("text and comment before root") do
      assert_eq(soap_json("pre<!--c-->more<a>x</a>"), "{\"a\":\"premorex\"}")
    end
    test("text before root with self-closing child") do
      assert_eq(soap_json("pre<a><b/></a>"), "{\"a\":{\"b\":null,\"_text\":\"pre\"}}")
    end
    test("text before root, nested text") do
      assert_eq(soap_json("pre<a>x<b>1</b></a>"), "{\"a\":{\"_text\":\"prex\",\"b\":\"1\"}}")
    end
    test("text after root") do
      assert_eq(soap_json("<a>1</a>junk"), "{\"a\":\"1\"}")
    end
    test("two roots") do
      assert_eq(soap_json("<a>1</a><b>2</b>"), "{\"a\":\"1\"}")
    end
    test("unmatched end after root") do
      assert_eq(soap_json("<a>1</a></b>"), "{\"a\":\"1\"}")
    end
    test("mismatched second root") do
      assert_eq(soap_json("<a>1</a><b></c>"), "{\"a\":\"1\"}")
    end
    test("doctype after root") do
      assert_eq(soap_json("<a>1</a><!DOCTYPE x>"), "{\"a\":\"1\"}")
    end
    test("unknown entity after root") do
      assert_eq(soap_json("<a>1</a>&bogus;"), "{\"a\":\"1\"}")
    end
    test("NUL charref after root") do
      assert_eq(soap_json("<a>1</a>&#0;"), "{\"a\":\"1\"}")
    end
    test("lone lt after root") do
      assert_eq(soap_json("<a>1</a><"), "{\"a\":\"1\"}")
    end
    test("unclosed second root") do
      assert_eq(soap_json("<a>1</a><b>"), "{\"a\":\"1\"}")
    end
    test("plain text") do
      assert_eq(soap_json("hello"), "null")
    end
    test("empty string") do
      assert_eq(soap_json(""), "null")
    end
    test("whitespace only") do
      assert_eq(soap_json("   \n "), "null")
    end
    test("comment only document") do
      assert_eq(soap_json("<!-- only comment -->"), "null")
    end
    test("declaration only") do
      assert_eq(soap_json("<?xml version=\"1.0\"?>"), "null")
    end
  end

  describe("truncated and malformed input") do
    test("truncated after child") do
      # fix: truncated input raises (bug 6)
      # was: {"r":{"a":"1"}}
      assert_eq(soap_json("<r><a>1</a>"), "ERR XML parsing error: <r> is not closed at the end of the input")
    end
    test("truncated deep") do
      # fix: truncated input raises (bug 6)
      # was: {"b":{"c":"2"}}
      assert_eq(soap_json("<r><a>1</a><b><c>2</c>"), "ERR XML parsing error: <b> is not closed at the end of the input")
    end
    test("truncated in text") do
      # fix: truncated input raises (bug 6)
      # was: {"r":{}}
      assert_eq(soap_json("<r>text"), "ERR XML parsing error: <r> is not closed at the end of the input")
    end
    test("truncated root only") do
      # fix: truncated input raises (bug 6)
      # was: {"r":{}}
      assert_eq(soap_json("<r>"), "ERR XML parsing error: <r> is not closed at the end of the input")
    end
    test("truncated tag") do
      assert_eq(soap_json("<r"), "ERR XML parsing error: syntax error: tag not closed: `>` not found before end of input")
    end
    test("truncated end tag") do
      assert_eq(soap_json("<r><a>1</a"), "ERR XML parsing error: syntax error: tag not closed: `>` not found before end of input")
    end
    test("truncated comment") do
      assert_eq(soap_json("<r><!-- x"), "ERR XML parsing error: syntax error: comment not closed: `-->` not found before end of input")
    end
    test("truncated cdata") do
      assert_eq(soap_json("<r><![CDATA[x"), "ERR XML parsing error: syntax error: CDATA not closed: `]]>` not found before end of input")
    end
    test("mismatched end") do
      assert_eq(soap_json("<a><b></a>"), "ERR XML parsing error: ill-formed document: expected `</b>`, but `</a>` was found")
    end
    test("unmatched end only") do
      assert_eq(soap_json("</a>"), "ERR XML parsing error: ill-formed document: close tag `</a>` does not match any open tag")
    end
    test("wrong end") do
      assert_eq(soap_json("<a></b>"), "ERR XML parsing error: ill-formed document: expected `</a>`, but `</b>` was found")
    end
    test("end tag with space") do
      assert_eq(soap_json("<a>x</a >"), "{\"a\":\"x\"}")
    end
    test("space after lt") do
      assert_eq(soap_json("< a>x</a>"), "ERR XML parsing error: ill-formed document: expected `</>`, but `</a>` was found")
    end
  end

  describe("attributes") do
    test("attributes dropped") do
      assert_eq(soap_json("<a x=\"1\"><b y=\"2\"/></a>"), "{\"a\":{\"b\":null}}")
    end
    test("attributes on leaf with text") do
      assert_eq(soap_json("<a x=\"1\">v</a>"), "{\"a\":\"v\"}")
    end
    test("duplicate attribute") do
      assert_eq(soap_json("<a x=\"1\" x=\"2\">t</a>"), "{\"a\":\"t\"}")
    end
    test("ampersand in attribute") do
      assert_eq(soap_json("<a x=\"AT&T\">v</a>"), "{\"a\":\"v\"}")
    end
    test("attribute without value") do
      assert_eq(soap_json("<a b>v</a>"), "{\"a\":\"v\"}")
    end
    test("NUL charref in attribute") do
      assert_eq(soap_json("<a x=\"&#0;\">v</a>"), "{\"a\":\"v\"}")
    end
    test("single and double quotes") do
      assert_eq(soap_json("<a x='1' y=\"2\">v</a>"), "{\"a\":\"v\"}")
    end
    test("attributes without space") do
      assert_eq(soap_json("<a x=\"1\"y=\"2\">v</a>"), "{\"a\":\"v\"}")
    end
    test("unquoted attribute") do
      assert_eq(soap_json("<a x=1>v</a>"), "{\"a\":\"v\"}")
    end
    test("lt in attribute") do
      assert_eq(soap_json("<a x=\"<\">v</a>"), "{\"a\":\"v\"}")
    end
    test("unknown entity in attribute") do
      assert_eq(soap_json("<a x=\"&foo;\">v</a>"), "{\"a\":\"v\"}")
    end
    test("unclosed attribute quote") do
      assert_eq(soap_json("<a x=\"1>v</a>"), "ERR XML parsing error: syntax error: attribute value not closed: `\"` not found before end of input")
    end
    test("2000 attributes") do
      assert_eq(soap_json("<a " + (0..2000).map { |i| "x#{i}=\"#{i}\"" }.join(" ") + ">v</a>"), "{\"a\":\"v\"}")
    end
  end

  describe("whitespace and line endings") do
    test("inner spaces kept") do
      assert_eq(soap_json("<a>  hello   world  </a>"), "{\"a\":\"hello   world\"}")
    end
    test("newlines trimmed at ends") do
      assert_eq(soap_json("<a>\n line1\n line2\n</a>"), "{\"a\":\"line1\\n line2\"}")
    end
    test("CRLF inside text") do
      # fix: line endings are normalised (CRLF)
      # was: {"a":"l1\r\nl2"}
      assert_eq(soap_json("<a>l1\r\nl2</a>"), "{\"a\":\"l1\\nl2\"}")
    end
    test("lone CR inside text") do
      # fix: line endings are normalised (CRLF)
      # was: {"a":"l1\rl2"}
      assert_eq(soap_json("<a>l1\rl2</a>"), "{\"a\":\"l1\\nl2\"}")
    end
    test("CRLF around text") do
      assert_eq(soap_json("<a>\r\n x \r\n</a>"), "{\"a\":\"x\"}")
    end
    test("CRLF in attribute") do
      assert_eq(soap_json("<a x=\"1\r\n2\">v</a>"), "{\"a\":\"v\"}")
    end
    test("CRLF between elements") do
      assert_eq(soap_json("<r>\r\n  <a>1</a>\r\n  <b>2</b>\r\n</r>"), "{\"r\":{\"a\":\"1\",\"b\":\"2\"}}")
    end
    test("tab only") do
      assert_eq(soap_json("<a>\t</a>"), "{\"a\":null}")
    end
    test("nbsp around text") do
      assert_eq(soap_json("<a>\u{a0}x\u{a0}</a>"), "{\"a\":\"x\"}")
    end
    test("nbsp only") do
      assert_eq(soap_json("<a>\u{a0}</a>"), "{\"a\":null}")
    end
    test("em space around comment") do
      assert_eq(soap_json("<a>x\u{2003}<!--c-->y</a>"), "{\"a\":\"x\u{2003}y\"}")
    end
    test("BOM before root") do
      assert_eq(soap_json("\u{feff}<a>x</a>"), "{\"a\":\"x\"}")
    end
    test("literal control char") do
      assert_eq(soap_json("<a>x\u{1}y</a>"), "{\"a\":\"x\\u0001y\"}")
    end
    test("pretty envelope") do
      assert_eq(soap_json("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\n  <soap:Body>\n    <m:GetPriceResponse xmlns:m=\"urn:x\">\n      <m:Price>1.90</m:Price>\n      <m:Name>Tea</m:Name>\n    </m:GetPriceResponse>\n  </soap:Body>\n</soap:Envelope>"), "{\"soap:Envelope\":{\"soap:Body\":{\"m:GetPriceResponse\":{\"m:Price\":\"1.90\",\"m:Name\":\"Tea\"}}}}")
    end
    test("soap fault") do
      # fix: whitespace around entities is kept (bug 1)
      # was: {"soap:Envelope":{"soap:Body":{"soap:Fault":{"faultcode":"soap:Server","faultstring":"Boom&bust","detail":null}}}}
      assert_eq(soap_json("<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><soap:Fault><faultcode>soap:Server</faultcode><faultstring>Boom &amp; bust</faultstring><detail/></soap:Fault></soap:Body></soap:Envelope>"), "{\"soap:Envelope\":{\"soap:Body\":{\"soap:Fault\":{\"faultcode\":\"soap:Server\",\"faultstring\":\"Boom & bust\",\"detail\":null}}}}")
    end
    test("wrap round trip") do
      assert_eq(soap_json(SOAP.wrap("<Ping>ok</Ping>")), "{\"soap:Envelope\":{\"soap:Body\":{\"Ping\":\"ok\"}}}")
    end
    test("escaped body envelope") do
      assert_eq(soap_json("<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body>&lt;b&gt;&amp;&lt;/b&gt;</soap:Body></soap:Envelope>"), "{\"soap:Envelope\":{\"soap:Body\":\"<b>&</b>\"}}")
    end
  end

  describe("declarations and doctype") do
    test("declaration with bare encoding") do
      assert_eq(soap_json("<?xml version=\"1.0\" encoding?><a>1</a>"), "{\"a\":\"1\"}")
    end
    test("declaration without version") do
      assert_eq(soap_json("<?xml encoding=\"UTF-8\"?><a>1</a>"), "{\"a\":\"1\"}")
    end
    test("declaration after root") do
      assert_eq(soap_json("<a>1</a><?xml version=\"1.0\"?>"), "{\"a\":\"1\"}")
    end
    test("declaration inside root") do
      assert_eq(soap_json("<a><?xml version=\"1.0\"?>1</a>"), "{\"a\":\"1\"}")
    end
    test("latin-1 declaration") do
      assert_eq(soap_json("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>1</a>"), "{\"a\":\"1\"}")
    end
    test("doctype") do
      assert_eq(soap_json("<!DOCTYPE x><a/>"), "ERR XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads")
    end
    test("doctype with entity") do
      assert_eq(soap_json("<!DOCTYPE x [<!ENTITY a \"b\">]><root>&a;</root>"), "ERR XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads")
    end
  end

  describe("depth and size limits") do
    test("depth 64") do
      assert_eq(soap_summary((0..64).map { |i| "<n#{i}>" }.join("") + "x" + (0..64).map { |i| "</n#{63 - i}>" }.join("")), "505:{\"n0\":{\"n1\":{\"n2\":{\"n3\":{\"n4\":{\"n5\":{\"n6\":{\"n7\":{\"n8\":{\"n9\":{\"n10\":{\"n11\":{\"n12\"")
    end
    test("depth 65") do
      assert_eq(soap_summary((0..65).map { |i| "<n#{i}>" }.join("") + "x" + (0..65).map { |i| "</n#{64 - i}>" }.join("")), "76:ERR XML parsing error: nesting depth exceeded 64 levels (likely DoS payload)")
    end
    test("depth 64 plus self-closing leaf") do
      assert_eq(soap_summary((0..64).map { |i| "<n#{i}>" }.join("") + "<leaf/>" + (0..64).map { |i| "</n#{63 - i}>" }.join("")), "515:{\"n0\":{\"n1\":{\"n2\":{\"n3\":{\"n4\":{\"n5\":{\"n6\":{\"n7\":{\"n8\":{\"n9\":{\"n10\":{\"n11\":{\"n12\"")
    end
    test("depth 65 self-closing leaf twice") do
      # fix: a self-closing repeat is appended (bug 3)
      # was: 515:{"n0":{"n1":{"n2":{"n3":{"n4":{"n5":{"n6":{"n7":{"n8":{"n9":{"n10":{"n11":{"n12"
      assert_eq(soap_summary((0..64).map { |i| "<n#{i}>" }.join("") + "<leaf/><leaf/>" + (0..64).map { |i| "</n#{63 - i}>" }.join("")), "522:{\"n0\":{\"n1\":{\"n2\":{\"n3\":{\"n4\":{\"n5\":{\"n6\":{\"n7\":{\"n8\":{\"n9\":{\"n10\":{\"n11\":{\"n12\"")
    end
    test("text of exactly 1 MiB") do
      assert_eq(soap_summary("<a>" + "A" * 1048576 + "</a>"), "1048584:{\"a\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    end
    test("text of 1 MiB plus one") do
      assert_eq(soap_summary("<a>" + "A" * 1048577 + "</a>"), "79:ERR XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)")
    end
    test("1 MiB split by a comment") do
      assert_eq(soap_summary("<a>" + "A" * 600000 + "<!--c-->" + "A" * 600000 + "</a>"), "79:ERR XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)")
    end
    test("1 MiB split by a child") do
      # fix: mixed-content runs are joined (bug 5), so the 1 MiB text cap now counts the element's whole text, not the run since the last child
      # was: 600026:{"a":{"_text":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
      assert_eq(soap_summary("<a>" + "A" * 600000 + "<c>1</c>" + "A" * 600000 + "</a>"), "79:ERR XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)")
    end
    test("1 MiB split by a self-closing child") do
      assert_eq(soap_summary("<a>" + "A" * 600000 + "<c/>" + "A" * 600000 + "</a>"), "79:ERR XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)")
    end
    test("1 MiB in two siblings") do
      assert_eq(soap_summary("<r><a>" + "A" * 600000 + "</a><b>" + "A" * 600000 + "</b></r>"), "1200021:{\"r\":{\"a\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    end
    test("1 MiB of entities") do
      assert_eq(soap_summary("<a>" + "&amp;" * 1048577 + "</a>"), "79:ERR XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)")
    end
    test("name of 5000 bytes") do
      # limit: names are at most 4096 bytes (MAX_NAME_LEN)
      # was: 5008:{"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn
      assert_eq(soap_summary("<" + "n" * 5000 + ">x</" + "n" * 5000 + ">"), "50:ERR XML parsing error: name longer than 4096 bytes")
    end
    test("attribute name of 5000 bytes") do
      assert_eq(soap_summary("<a " + "n" * 5000 + "=\"1\">x</a>"), "9:{\"a\":\"x\"}")
    end
    test("10000 siblings") do
      assert_eq(soap_summary("<r>" + "<i>1</i>" * 10000 + "</r>"), "40013:{\"r\":{\"i\":[\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"1\",\"")
    end
  end
end
