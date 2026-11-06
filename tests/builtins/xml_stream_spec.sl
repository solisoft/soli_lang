# Xml streaming: `Xml.each(path, match, ns = nil) do |node| … end` and
# `Xml.reader(path, match, ns = nil)` (XmlReader). Files only; each match is a
# detached node carrying the namespaces in scope where it stood. The file jail
# (under `soli serve` and jobs) is covered by tests/xml_stream_jail_test.rs,
# memory on a large file by tests/xml_stream_memory_test.rs.

const FEED = "tests/builtins/_fixtures/xml/feed.rss"
SPEC_DIR = "/tmp/soli_xml_stream_spec"

def spec_path(name)
  "#{SPEC_DIR}/#{name}"
end

def write_xml(name, body)
  path = spec_path(name)
  barf(path, body)
  path
end

# A file of `chunks` × `chunk` bytes between `head` and `tail`, written a
# piece at a time.
def write_big(name, head, chunk, chunks, tail)
  path = spec_path(name)
  barf(path, head)
  chunks.times do
    File.append(path, chunk)
  end
  File.append(path, tail)
  path
end

# The number of matches `Xml.each` yields.
def count_each(path, expr, ns = nil)
  Xml.each(path, expr, ns) do |node|
    nil
  end
end

const CATALOG = """<catalog>
  <product sku="a1" status="active"><name>Lamp</name></product>
  <product sku="b2" status="retired"><name>Desk</name></product>
  <group>
    <product sku="c3" status="active"><name>Chair</name><product sku="inner"/></product>
  </group>
</catalog>"""

const ATOM = """<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="urn:media">
  <entry><title>One</title><media:thumb url="1.png"/></entry>
  <entry><title>Two</title></entry>
</feed>"""

describe("Xml streaming") do
  before_each() do
    System.run_sync(["rm", "-rf", SPEC_DIR])
    mkdir_p(SPEC_DIR)
  end

  after_each() do
    System.run_sync(["rm", "-rf", SPEC_DIR])
  end

  describe("Xml.each") do
    test("yields each match and returns the count") do
      names = []
      count = Xml.each(FEED, "item") do |item|
        names.push(item.children[0].text)
      end
      assert_eq(count, 2)
      assert_eq(names, ["First", "Second"])
    end

    test("a match is a detached XmlNode") do
      seen = []
      Xml.each(FEED, "item") do |item|
        seen.push(item)
      end
      first = seen[0]
      assert_eq(type(first), "XmlNode")
      assert_eq(first.name, "item")
      assert_eq(first["id"], "1")
      assert_null(first.parent)
      assert_null(first.document)
    end

    test("a match carries the namespaces in scope") do
      xmls = []
      Xml.each(FEED, "item") do |item|
        xmls.push(item.to_xml)
      end
      assert(xmls[0].starts_with?("<item xmlns:dc=\"http://purl.org/dc/elements/1.1/\" id=\"1\">"))
      creators = []
      Xml.each(FEED, "dc:creator") do |creator|
        creators.push(creator.text)
        assert_eq(creator.namespace, "http://purl.org/dc/elements/1.1/")
      end
      assert_eq(creators, ["Ada", "Grace"])
    end

    test("matches only the outermost element") do
      path = write_xml("catalog.xml", CATALOG)
      skus = []
      Xml.each(path, "product") do |product|
        skus.push(product["sku"])
      end
      assert_eq(skus, ["a1", "b2", "c3"])
    end

    test("chains, anchors and the final predicate") do
      path = write_xml("catalog.xml", CATALOG)
      assert_eq(count_each(path, "catalog/product"), 2)
      assert_eq(count_each(path, "/catalog/group/product"), 1)
      assert_eq(count_each(path, "/group/product"), 0)
      assert_eq(count_each(path, "//group/*"), 1)
      active = []
      Xml.each(path, "product[@status='active']") do |product|
        active.push(product["sku"])
      end
      assert_eq(active, ["a1", "c3"])
      assert_eq(count_each(path, "product[@status!='active']"), 1)
      assert_eq(count_each(path, "product[@status]"), 3)
    end

    test("namespaces: lenient names, a table, the root's prefixes") do
      path = write_xml("atom.xml", ATOM)
      assert_eq(count_each(path, "entry"), 2)
      assert_eq(count_each(path, "entry", {"": "urn:other"}), 0)
      assert_eq(count_each(path, "a:entry", {"a": "http://www.w3.org/2005/Atom"}), 2)
      assert_eq(count_each(path, "xmlns:entry"), 2)
      urls = []
      Xml.each(path, "media:thumb") do |thumb|
        urls.push(thumb["url"])
      end
      assert_eq(urls, ["1.png"])
    end

    test("the match is a node like any other") do
      path = write_xml("catalog.xml", CATALOG)
      rows = []
      Xml.each(path, "product[@status='active']") do |product|
        rows.push(product.to_h["product"])
      end
      assert_eq(rows[0]["@sku"], "a1")
      assert_eq(rows[0]["name"], "Lamp")
    end

    test("find and xpath work on a match, namespaces included") do
      found = []
      Xml.each(FEED, "item") do |item|
        found.push([
          item.find("title").text,
          item.at_xpath("dc:creator/text()"),
          item.xpath("/item/@id")[0],
          item.find_all("*").length
        ])
      end
      assert_eq(found, [["First", "Ada", "1", 3], ["Second", "Grace", "2", 2]])
    end

    test("a throw inside the block reaches catch and closes the file") do
      seen = 0
      caught = nil
      try
        Xml.each(FEED, "item") do |item|
          seen = seen + 1
          throw "stop at #{item["id"]}"
        end
      catch e
        caught = e
      end
      assert_eq(seen, 1)
      assert(caught.to_s.includes?("stop at 1"))
    end

    test("break in the block acts like next") do
      seen = []
      count = Xml.each(FEED, "item") do |item|
        break if item["id"] == "1"
        seen.push(item["id"])
      end
      assert_eq(count, 2)
      assert_eq(seen, ["2"])
    end

    test("a function value works as the block") do
      ids = []
      collect = fn(node) { ids.push(node["id"]) }
      assert_eq(Xml.each(FEED, "item", &collect), 2)
      assert_eq(ids, ["1", "2"])
    end

    test("without a block it raises") do
      assert_raises("Xml.each expects a block") do
        Xml.each(FEED, "item")
      end
    end

    test("refuses a missing file, a directory and a non-String path") do
      assert_raises("Xml.each: cannot open") do
        count_each(spec_path("missing.xml"), "item")
      end
      assert_raises("not a file") do
        count_each(SPEC_DIR, "item")
      end
      assert_raises("Xml.each: the path must be a String") do
        count_each(42, "item")
      end
    end

    test("refuses a FIFO without blocking on it") do
      fifo = spec_path("pipe.xml")
      System.run_sync(["mkfifo", fifo])
      assert_raises("Xml.each: cannot read #{fifo}: not a file") do
        count_each(fifo, "item")
      end
      assert_raises("Xml.reader: cannot read #{fifo}: not a file") do
        Xml.reader(fifo, "item")
      end
      assert_raises("Xml.parse_file: cannot read #{fifo}: not a file") do
        Xml.parse_file(fifo)
      end
      assert_raises("Xml.parse_file: cannot read #{SPEC_DIR}: not a file") do
        Xml.parse_file(SPEC_DIR)
      end
    end

    test("refuses a match streaming cannot do, naming the column") do
      assert_raises("Xml.each: unsupported match at column 6: only [@a]") do
        count_each(FEED, "item[1]")
      end
      assert_raises("only allowed on the last step") do
        count_each(FEED, "channel[@x]/item")
      end
      assert_raises("Xml.each: unbound namespace prefix 'zz'") do
        count_each(FEED, "zz:item")
      end
    end

    test("an error past the last match still raises, with line and column") do
      path = write_xml("bad.xml", "<r>\n  <item/>\n  <item></r>")
      seen = 0
      assert_raises("Xml.each: line 3, column 9: expected </item>, found </r>") do
        Xml.each(path, "item") do |n|
          seen = seen + 1
        end
      end
      assert_eq(seen, 1)
    end
  end

  describe("hostile input") do
    test("a DOCTYPE is refused") do
      path = write_xml("doctype.xml", "<!DOCTYPE r [<!ENTITY a \"aaaa\">]><r><item>&a;</item></r>")
      assert_raises("Xml.each: line 1, column 1: DOCTYPE declarations are not allowed in XML documents") do
        count_each(path, "item")
      end
    end

    test("doctype: \"skip\" reads past a DOCTYPE and still declares nothing") do
      path = write_xml("doctype_skip.xml", "<!DOCTYPE r SYSTEM \"https://example.com/r.dtd\"><r><item>1</item><item>2</item></r>")
      seen = []
      total = Xml.each(path, "item", doctype: "skip") do |node|
        seen.push(node.text)
      end
      assert_eq(total, 2)
      assert_eq(seen, ["1", "2"])
      reader = Xml.reader(path, "item", nil, {"doctype": "skip"})
      assert_eq(reader.next.text, "1")
      reader.close
      entity = write_xml("doctype_entity.xml", "<!DOCTYPE r [<!ENTITY a \"aaaa\">]><r><item>&a;</item></r>")
      assert_raises("unknown entity '&a;'") do
        Xml.each(entity, "item", doctype: "skip") do |node|
          nil
        end
      end
      assert_raises("option 'doctype' must be") do
        Xml.reader(path, "item", doctype: "maybe")
      end
    end

    test("nesting deeper than SOLI_MAX_XML_DEPTH is refused") do
      path = write_xml("deep.xml", "<a>" * 300 + "</a>" * 300)
      assert_raises("more than 256 nested elements (SOLI_MAX_XML_DEPTH)") do
        count_each(path, "item")
      end
    end

    test("a text run past SOLI_MAX_XML_BYTES is refused before it is read") do
      # 72 MiB of text inside a skipped element: one event over the 64 MiB cap.
      path = write_big("text.xml", "<r><skip>", "x" * (8 * 1024 * 1024), 9, "</skip><item/></r>")
      assert_raises("exceeds the 67108864-byte limit (SOLI_MAX_XML_BYTES)") do
        count_each(path, "item")
      end
    end

    test("an attribute past SOLI_MAX_XML_BYTES is refused") do
      path = write_big("attr.xml", "<r><item a=\"", "x" * (8 * 1024 * 1024), 9, "\"/></r>")
      assert_raises("exceeds the 67108864-byte limit (SOLI_MAX_XML_BYTES)") do
        count_each(path, "item")
      end
    end
  end

  describe("Xml.reader") do
    test("next returns each match, then nil") do
      reader = Xml.reader(FEED, "item")
      assert_eq(type(reader), "XmlReader")
      first = reader.next
      assert_eq(first["id"], "1")
      assert_eq(reader.next["id"], "2")
      assert_null(reader.next)
      assert_null(reader.next)
      assert_eq(reader.count, 2)
    end

    test("break in a while loop really stops") do
      reader = Xml.reader(FEED, "item")
      found = nil
      while (item = reader.next)
        if item["id"] == "1"
          found = item
          break
        end
      end
      assert_eq(found["id"], "1")
      assert_eq(reader.count, 1)
      reader.close
      assert_null(reader.next)
    end

    test("shows its path") do
      reader = Xml.reader(FEED, "item")
      assert_eq("#{reader}", "<XmlReader #{FEED}>")
    end

    test("has no constructor and no writable property") do
      assert_raises("XmlReader has no constructor: use Xml.reader(path, match)") do
        XmlReader.new
      end
      reader = Xml.reader(FEED, "item")
      assert_raises("XmlReader has no writable property 'foo'") do
        reader.foo = 1
      end
      assert_null(reader["__xml"])
    end

    test("an error is raised once, then the reader is closed") do
      path = write_xml("bad.xml", "<r><item/><item></r>")
      reader = Xml.reader(path, "item")
      assert_eq(reader.next.name, "item")
      assert_raises("Xml.reader: line 1, column 17: expected </item>, found </r>") do
        reader.next
      end
      assert_null(reader.next)
    end

    test("refuses a block") do
      assert_raises("Xml.reader takes no block") do
        Xml.reader(FEED, "item") do |n|
          nil
        end
      end
    end
  end
end
