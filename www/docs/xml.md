# XML

Soli parses, queries, builds and streams XML with a built-in Rust parser. A
document is a tree of node objects: `doc.root`, `node["id"]`, `node.text`,
`doc.find("channel/title")`, `doc.to_xml`. There is nothing to install or
import.

Four classes are involved:

| Class | What it is |
|---|---|
| `Xml` | The entry points: `Xml.parse`, `Xml.parse_file`, `Xml.document`, `Xml.element`, `Xml.build`, `Xml.each`, `Xml.reader`, `Xml.well_formed?`, `Xml.escape`. It also holds the XML-signature helpers (`Xml.c14n_exclusive`, …), documented under [XML Signatures](/docs/builtins/xml-signatures). |
| `XmlNode` | Every node: elements, text, CDATA, comments, processing instructions. `node.kind` tells them apart. |
| `XmlDocument` | The document node, a subclass of `XmlNode`: it has every node method plus `root`, `declaration` and `doctype`. |
| `XmlReader` | A pull cursor over a file, from `Xml.reader`. |

None of them has a constructor: `XmlNode.new`, `XmlNode.new(...)` and
`XmlNode()` raise `XmlNode has no constructor: use Xml.parse, Xml.element or
node.add`, and the same goes for `XmlDocument` and `XmlReader`.

## Quick start

```soli
body = """<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>News &amp; views</title>
    <item id="1"><title>First</title><link>https://example.com/1</link></item>
    <item id="2"><title>Second</title></item>
  </channel>
</rss>"""

doc = Xml.parse(body)
doc.root.name                       # => "rss"
doc.root["version"]                 # => "2.0"
doc.find("channel/title").text      # => "News & views"
doc.find_all("channel/item").each do |item|
  print("#{item["id"]}: #{item.find("title").text}")
end
# 1: First
# 2: Second
```

## Parsing

### Xml.parse(xml, options = nil)

Parses a String and returns an `XmlDocument`. It raises on malformed input.
`Xml.parse` takes a String only and never treats its argument as a path, so a
request body can never turn into a file read.

```soli
doc = Xml.parse(body)
doc = Xml.parse(body, strip_blank: true)        # drop whitespace-only text nodes
doc = Xml.parse(body, {"max_depth": 32})        # options as one Hash work too
doc = Xml.parse(body, doctype: "skip")          # read past a <!DOCTYPE …> (see below)
```

| Option | Default | Effect |
|---|---|---|
| `strip_blank` | `false` | Drop text nodes that hold only whitespace. Off by default, so the tree is lossless. |
| `max_depth` | `SOLI_MAX_XML_DEPTH` (256) | Deepest element nesting accepted. |
| `max_nodes` | `SOLI_MAX_XML_NODES` (1,000,000) | Most nodes in the tree. |
| `max_bytes` | `SOLI_MAX_XML_BYTES` (64 MiB) | Largest input. |
| `doctype` | `"refuse"` | `"skip"` reads past one `<!DOCTYPE …>` instead of raising (see [DOCTYPE](#doctype)). |

The three limits can only be **lowered** by an option, never raised: raise the
environment variable instead. An unknown option raises
`Xml.parse: unknown option 'deep' (expected one of: strip_blank, max_depth, max_nodes, max_bytes, doctype)`.

What the parser does with the input:

- **UTF-8 only.** A byte-order mark is skipped, the `encoding=` declaration is
  ignored, and invalid UTF-8 raises.
- **Line endings** are normalised (CRLF and a lone CR become LF) before
  attribute values are normalised, so a CRLF inside an attribute becomes one
  space.
- The five predefined entities (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;`) and
  character references (`&#233;`, `&#xE9;`) are decoded. A reference to a
  character XML forbids (`&#0;`) raises.
- Text broken up by entity references is joined into one text node. CDATA stays
  its own node kind.
- It raises on: a `DOCTYPE` (unless `doctype: "skip"`), an unknown entity
  (`&nbsp;`), an unbound namespace prefix, a duplicate attribute, a mismatched
  or unclosed tag, no root element, a second root element, and non-whitespace
  text outside the root.

#### DOCTYPE

A `<!DOCTYPE …>` raises by default:
`Xml.parse: line 2, column 1: DOCTYPE declarations are not allowed in XML documents`.
Some APIs put one on every response all the same — Qualys v2, PingCastle and
ORCQ reports, Apple property lists. `doctype: "skip"` reads past it instead of
raising, so there is no need to cut it out of the body by hand:

```soli
body = """<?xml version="1.0" encoding="UTF-8" ?>
<!DOCTYPE SIMPLE_RETURN SYSTEM "https://qualysapi.qualys.com/api/2.0/simple_return.dtd">
<SIMPLE_RETURN><RESPONSE><TEXT>Scan launched</TEXT></RESPONSE></SIMPLE_RETURN>"""

doc = Xml.parse(body, doctype: "skip")
doc.find("//TEXT").text   # => "Scan launched"
doc.doctype               # => "SIMPLE_RETURN"
```

- **Nothing of the DOCTYPE is used.** The external id (`SYSTEM "…"`,
  `PUBLIC "…" "…"`) is never fetched, and an internal subset (`[ … ]`) is read
  past without being interpreted — quoted strings, comments and processing
  instructions holding `]` or `>` included. No entity is ever declared, so
  `&name;` for an entity the subset declares still raises
  `unknown entity '&name;'`: skipping opens no door to XXE or "billion
  laughs". Default attribute values from an `<!ATTLIST>` are not applied
  either.
- **Only its name is kept**: `doc.doctype` is `"SIMPLE_RETURN"` above, and
  `nil` for a document without one (or built with `Xml.document`). `to_xml`
  never writes the DOCTYPE back.
- **One DOCTYPE, before the root element.** A second one, or one after the
  root starts, raises
  `a DOCTYPE is only allowed once, before the root element`. The XML
  declaration still has to come first.
- **The limits still hold.** The DOCTYPE counts toward `max_bytes`, so a huge
  internal subset raises like any oversized input.
- The only values are `"refuse"` (the default; `nil` too) and `"skip"`;
  anything else raises
  `Xml.parse: option 'doctype' must be "refuse" or "skip", got "ignore"`.
- `Xml.parse_file`, `Xml.each` and `Xml.reader` take the same option.
  `Xml.well_formed?`, `SOAP.parse` / `SOAP.call` and the
  [XML-signature](/docs/builtins/xml-signatures) helpers always refuse a
  DOCTYPE.

### Xml.parse_file(path, options = nil)

Reads a file and parses it, with the same options. The path goes through the
same checks as `File.read`: under `soli serve` and in jobs it must stay inside
the application root, a symbolic link is not followed, and a directory, FIFO or
device is refused (`Xml.parse_file: cannot read imports: not a file`) without
blocking. The file size is checked against `max_bytes` before anything is read.

```soli
invoice = Xml.parse_file("imports/invoice.xml")
```

### Xml.well_formed?(xml)

`true` if the String parses, `false` otherwise (including for `nil`). It never
raises.

```soli
Xml.well_formed?("<a/>")         # => true
Xml.well_formed?("<a><b></a>")   # => false
```

### Xml.escape(text)

Escapes the five XML characters, so the result is safe both in text and inside
a quoted attribute value. `Xml.escape(nil)` is `""`. The output is byte for
byte the same as `SOAP.xml_escape`.

```soli
Xml.escape("Tom & \"Jerry\" <3")   # => "Tom &amp; &quot;Jerry&quot; &lt;3"
```

You rarely need it: the builder escapes everything it writes.

## Nodes

### Kinds and names

| Member | On an element | On other nodes |
|---|---|---|
| `kind` | `"element"` | `"text"`, `"cdata"`, `"comment"`, `"pi"` or `"document"` |
| `element?`, `text?`, `cdata?`, `comment?`, `pi?`, `document?` | Bool | Bool |
| `name` | the qualified name as written: `"inv:Invoice"` | a processing instruction's target; otherwise `nil` |
| `local_name` | `"Invoice"` | `nil` |
| `prefix` | `"inv"`, or `nil` | `nil` |
| `namespace` | the namespace URI, or `nil` | `nil` |
| `namespaces` | Hash of every prefix in scope → URI (`""` is the default namespace) | `{}` |
| `namespace_declarations` | Hash of the declarations made on this element only | `{}` |
| `path` | `"/rss/channel/item[2]"` | the parent's path plus `/text()[k]` |

`doc.declaration` is the XML declaration as a Hash, or `nil` when the document
has none:

```soli
doc.declaration   # => {"version": "1.0", "encoding": "UTF-8", "standalone": nil}
```

`doc.doctype` is the name of the DOCTYPE that `doctype: "skip"` read past
(`"SIMPLE_RETURN"` for `<!DOCTYPE SIMPLE_RETURN SYSTEM "…">`), or `nil`.

`type(node)` and `node.class` give `"XmlNode"` or `"XmlDocument"`. `is_a?`
takes the class name as a String: `node.is_a?("XmlNode")`.

### Navigation

Every method that returns an Array returns a fresh one: changing it does not
change the tree.

| Member | Returns |
|---|---|
| `doc.root` | the root element (`nil` only on a document built without one) |
| `children` | the **element** children, in document order |
| `nodes` | **every** child node (text, CDATA, comments and PIs included). On a document: what comes before the root, the root, what comes after. |
| `parent` | the parent element, or `nil` for the root, a detached node or a streamed match. Reach the document with `document`. |
| `document` | the `XmlDocument`, or `nil` for a detached node or a streamed match |
| `next_sibling`, `previous_sibling` | the next or previous **element** sibling, or `nil` |
| `next_node`, `previous_node` | the next or previous sibling of any kind, or `nil` |
| `ancestors` | Array of elements, nearest first |
| `descendants` | Array of every element below, in document order |

```soli
channel = doc.root.children[0]
channel.children.map { |child| child.name }   # => ["title", "item", "item"]
channel.nodes.map { |node| node.kind }        # => ["text", "element", "text", "element", "text", "element", "text"]
first = channel.children[1]
first.path                                    # => "/rss/channel/item[1]"
first.next_sibling["id"]                      # => "2"
first.ancestors.map { |node| node.name }      # => ["channel", "rss"]
first.document == doc                         # => true
```

The whitespace between tags is kept as text nodes, which is why `nodes` is
longer than `children`. Parse with `strip_blank: true` to drop it.

### Text

```soli
para = Xml.parse("<p>Hello <b>big</b> world &amp; more</p>").root
para.text        # => "Hello big world & more"       (all the text below, decoded)
para.own_text    # => "Hello  world & more"          (direct text and CDATA children only)
para.inner_xml   # => "Hello <b>big</b> world &amp; more"
```

`text` is the XPath string-value: every descendant text and CDATA node joined,
entities decoded, **not** trimmed. On a text, CDATA or comment node it is the
content; on a processing instruction, its data.

### Attributes

```soli
link = Xml.parse("<a href=\"/x\" xmlns:xl=\"http://www.w3.org/1999/xlink\" xl:title=\"T\"/>").root
link["href"]                                            # => "/x"
link.get("href")                                        # => "/x", the same thing
link["missing"]                                         # => nil
link.attribute("title", "http://www.w3.org/1999/xlink") # => "T", by namespace URI
link.attributes                                         # => {"href": "/x", "xl:title": "T"}
link.has_attribute?("href")                             # => true
```

`node["name"]` looks an attribute up by its name as written (`"xl:title"`).
`attributes` keeps source order and leaves out `xmlns` declarations. On a node
that is not an element, `node["x"]` is `nil` and `attributes` is `{}`.

## Queries: find and xpath

There are two pairs of query methods. They share one grammar and one
evaluator, and differ in exactly two ways:

| | `find(expr, ns = nil)` / `find_all(expr, ns = nil)` | `at_xpath(expr, ns = nil)` / `xpath(expr, ns = nil)` |
|---|---|---|
| Flavour | lenient, like Python's ElementTree | strict XPath 1.0, with Nokogiri's method names |
| A relative path on a **document** | starts at the **root element**: `doc.find("channel/item")` | starts at the document node: `doc.xpath("rss/channel/item")` |
| A relative path on an element | starts at the element | starts at the element |
| An unprefixed name | matches that local name **in any namespace** | matches only elements in **no namespace** |
| A miss | `find` gives `nil`, `find_all` gives `[]` | `at_xpath` gives `nil`, `xpath` gives `[]` |

Absolute paths (`/rss/channel`) and `//name` start at the document in both
flavours. A consequence of the first row: `doc.find("rss")` is `nil` (it looks
for an `rss` child of the root), while `doc.xpath("rss")` finds the root.

Results come in document order without duplicates. Elements, comments and
other nodes come back as `XmlNode`; a final `@attr` or `text()` step gives
Strings:

```soli
doc.find_all("channel/item/@id")           # => ["1", "2"]
doc.find("channel/item/title/text()")      # => "First"
doc.at_xpath("/rss/channel/item[2]")["id"] # => "2"
```

A node a query returns is the same node navigation reaches: `==` holds between
them, and changing one changes the tree.

### Namespaces in queries

Atom puts every element in a default namespace. `find` reads it without any
setup; `xpath` asks you to name the namespace:

```soli
atom = """<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
  <title>Blog</title>
  <entry><title>One</title><link rel="alternate" href="/1"/><media:thumbnail url="1.png"/></entry>
  <entry><title>Two</title><link rel="alternate" href="/2"/></entry>
</feed>"""
feed = Xml.parse(atom)

feed.find_all("entry").length                          # => 2
feed.find_all("entry/link[@rel='alternate']/@href")    # => ["/1", "/2"]

feed.xpath("//entry")                                  # => [] (no namespace)
feed.xpath("//xmlns:entry").length                     # => 2, the root's default namespace
feed.xpath("//*:entry").length                         # => 2, any namespace
feed.xpath("//a:entry/a:title/text()", {"a": "http://www.w3.org/2005/Atom"})   # => ["One", "Two"]
feed.xpath("//entry", {"": "http://www.w3.org/2005/Atom"}).length              # => 2
feed.at_xpath("//a:entry[2]/a:title/text()", a: "http://www.w3.org/2005/Atom") # => "Two"
```

- A prefix in an expression is resolved, in order, through: the `ns` table
  passed to the call (a Hash, or named arguments); the namespace declarations
  **on the root element**; `xmlns:`, bound to the root's default namespace;
  and `xml`, which is always bound. Otherwise the query raises
  `Xml.xpath: unbound prefix 'q'`. Prefixes are compared by the URI they
  resolve to, never as text, so any prefix that names the right URI works.
- The `""` key of the table sets the namespace unprefixed names match, in both
  flavours.
- `*:name` matches a local name in any namespace; `p:*` matches every element
  in one namespace.
- The fallback to the root's declarations only sees the root. A prefix
  declared lower down, or declared on the root and re-declared lower down with
  a different URI, needs the table, which always wins.

### The XPath subset

```
Expr      := Path ('|' Path)*
Path      := '/' RelPath? | '//' RelPath | RelPath
RelPath   := Step (('/' | '//') Step)*
Step      := '.' | '..' | '@' (QName | '*') | NodeTest Predicate*
NodeTest  := QName | '*' | prefix ':*' | '*:' name
           | 'text()' | 'node()' | 'comment()' | 'processing-instruction(' 'target'? ')'
Predicate := '[' (Integer | 'last()' | Or) ']'
Or        := And ('or' And)*
And       := Test ('and' Test)*
Test      := 'not(' Or ')' | '(' Or ')'
           | Operand (('=' | '!=') Literal)?
           | 'contains(' Operand ',' Literal ')' | 'starts-with(' Operand ',' Literal ')'
           | 'position()' ('=' | '!=' | '<' | '>' | '<=' | '>=') Integer
Operand   := '@' QName | QName | 'text()' | '.'
Literal   := 'single-quoted' | "double-quoted"
```

```soli
shop = Xml.parse("<shop><p sku=\"a\" stock=\"0\"><n>Lamp</n></p><p sku=\"b\" stock=\"3\"><n>Desk</n><tag>sale</tag></p><p sku=\"c\" stock=\"5\"><n>Chair</n></p></shop>")
shop.find_all("p[@stock!='0']/@sku")                   # => ["b", "c"]
shop.find_all("p[tag='sale']/n/text()")                # => ["Desk"]
shop.find("p[2]")["sku"]                               # => "b"
shop.find("p[last()]")["sku"]                          # => "c"
shop.find_all("p[position() > 1]/@sku")                # => ["b", "c"]
shop.find_all("p[not(tag)]/@sku")                      # => ["a", "c"]
shop.find_all("p[@sku='a' or @sku='c']/n/text()")      # => ["Lamp", "Chair"]
shop.find_all("p[contains(n, 'ai') and starts-with(@sku, 'c')]/@sku")   # => ["c"]
shop.xpath("//p/n | //p/tag").map { |node| node.name } # => ["n", "n", "tag", "n"]
```

- `[n]` is 1-based and counts per parent, as in XPath: `//b[1]` is the first
  `b` of **each** parent. `last()` is the last of that set.
- Comparisons compare strings, and are true if **any** match: `[tag='x']` is
  true if any `tag` child's text is `x`. `[@a='v']` compares the attribute,
  `[text()='x']` the direct text runs, `[.='x']` the node's own text. A bare
  operand (`[@a]`, `[tag]`) tests that it exists.
- `@attr` and `text()` may only be the last step of a path.

Left out, and refused when the expression is compiled, with the column:

- axes written `axis::` (`child::a`), variables and arithmetic;
- every function except the ones above (`Xml.xpath: unsupported XPath at column 1: function 'sum'`);
- comparing numbers, except with `position()` (`//p[@stock > 1]`);
- a path inside a predicate: an operand is one child step or attribute
  (`//p[n/text()='Lamp']`; write `//p[n='Lamp']`);
- a parenthesised path: for `(//a)[1]` write `at_xpath("//a")`.

There are two kinds of error, `invalid XPath` (a syntax error) and
`unsupported XPath` (valid XPath this subset leaves out). `find` and
`find_all` errors start with `Xml.find:`, the others with `Xml.xpath:`.

A **literal** expression is compiled when the file is checked, so `soli check`
(and the check `soli` runs before a script) refuses `doc.xpath("sum(//p)")`
before the program starts. An expression built at run time is compiled when
it runs.

### Query limits

- An expression is at most 4096 bytes, 64 steps, 8 predicates per step, 16
  `and`/`or` terms per predicate, and 16 levels of `not(…)` and parentheses.
  Past one: `XPath too complex: more than 64 steps`.
- One evaluation visits at most `SOLI_MAX_XPATH_VISITS` nodes (10,000,000 by
  default), plus one visit per 64 bytes of text it copies to compare. Past it:
  `Xml.xpath: evaluation exceeded 10000000 node visits (SOLI_MAX_XPATH_VISITS)`.
  A step visits each node at most once, so an ordinary query costs at most
  steps × nodes.
- Compiled expressions are cached (128 per thread), so a query in a loop is
  compiled once.

## Building

### Xml.document, Xml.element and node.add

```soli
posts = [
  {"id": 1, "title": "XML lands", "url": "https://example.com/1", "body_html": "<p>Hi</p>"},
  {"id": 2, "title": "Tom & Jerry", "url": "https://example.com/2", "body_html": "<p>Yo</p>"}
]

doc = Xml.document("rss", version: "2.0")
channel = doc.root.add("channel")
channel.add("title", "News & views")
posts.each do |post|
  item = channel.add("item", id: post.id)
  item.add("title", post.title)
  item.add("link", post.url, rel: "alternate")
  item.add_cdata(post.body_html)
end
print(doc.to_xml(indent: 2))
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>News &amp; views</title>
    <item id="1"><title>XML lands</title><link rel="alternate">https://example.com/1</link><![CDATA[<p>Hi</p>]]></item>
    <item id="2"><title>Tom &amp; Jerry</title><link rel="alternate">https://example.com/2</link><![CDATA[<p>Yo</p>]]></item>
  </channel>
</rss>
```

(`indent` only indents elements whose content is all elements, so the mixed
`item`s stay on one line.)

| Call | Returns |
|---|---|
| `Xml.document(root_name = nil, attrs = nil)` | a new `XmlDocument`, with a root element when `root_name` is given. It writes an XML declaration by default. |
| `Xml.element(name, content = nil, attrs = nil)` | a detached element in a tree of its own |
| `node.add(name, content = nil, attrs = nil)` | appends a new element child to `node` and returns the **child** |

`content` and `attrs` follow the same rules everywhere:

- `content` is a String, Int, Float, Bool or Decimal (written with `.to_s`), an
  `XmlNode`, or an Array of these.
- A Hash in the content position is taken as the attributes, so
  `add("item", id: 1)` and `add("item", {"id": 1})` both work. Named arguments
  are attributes.
- An attribute whose value is `nil` is left out.
- An `xmlns` or `xmlns:p` key declares a namespace.
- A prefixed name (`inv:Line`) must be bound by a declaration in scope when
  the node is created, or the call raises
  `XmlNode.add: unbound namespace prefix 'cbc'`.

```soli
cell = Xml.element("td", ["Total: ", Xml.element("b", 42)], {"class": "sum"})
cell.to_xml                                   # => "<td class=\"sum\">Total: <b>42</b></td>"
Xml.element("x", {"k": "v", "skip": nil}).to_xml   # => "<x k=\"v\"/>"

invoice = Xml.document("inv:Invoice", {"xmlns:inv": "urn:inv"})
invoice.root.add("inv:Line", "1").namespace   # => "urn:inv"
```

`add` is all-or-nothing: if its content fails (for example a node that would
become its own ancestor), no element is left behind.

### Blocks: Xml.build and node.add … do

`Xml.build(root_name, attrs = nil) do |root| … end` creates a document, runs
the block with its root element, and returns the document (with a
declaration). `node.add(name, content = nil, attrs = nil) do |child| … end`
appends the child, runs the block with it, and returns the child.

```soli
feed = Xml.build("feed", {"xmlns": "http://www.w3.org/2005/Atom"}) do |root|
  root.add("title", "Blog")
  posts.each do |post|
    root.add("entry") do |entry|
      entry.add("title", post.title)
      entry.add("link", nil, href: post.url)
    end
  end
end
print(feed.to_xml(indent: 2))
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Blog</title>
  <entry>
    <title>XML lands</title>
    <link href="https://example.com/1"/>
  </entry>
  <entry>
    <title>Tom &amp; Jerry</title>
    <link href="https://example.com/2"/>
  </entry>
</feed>
```

- The block needs a parameter (`|entry|`). A lambda works too:
  `root.add("a", &builder)`, or passed as the last argument.
- The block sees and updates the variables around it. An error raised inside
  it propagates. The request deadline is checked before each block runs.
- `return`, `break` and `next` inside a builder block only leave that block.
- `&.` works with named arguments and blocks, and evaluates neither on `nil`:
  `parent&.add("x", id: 1) do |x| … end`.

### Other kinds of node

```soli
root = Xml.document("r").root
root.add_text("a < b")
root.add_comment("generated")
root.add_pi("xml-stylesheet", "href=\"style.xsl\"")
root.to_xml   # => "<r>a &lt; b<!--generated--><?xml-stylesheet href=\"style.xsl\"?></r>"
```

`add_text(s)`, `add_cdata(s)`, `add_comment(s)` and `add_pi(target, data = "")`
each append one node and return it. A CDATA section containing `]]>` is split
into two sections when written.

### Validation

- Names are checked as XML qualified names: `Xml.element("1bad")` raises
  `Xml.element: invalid XML name '1bad'`.
- Characters XML forbids in text and attribute values (the C0 controls other
  than tab, LF and CR, plus U+FFFE and U+FFFF) are **dropped**, so a stray
  control character in a database row cannot make the output unreadable:
  `Xml.element("t", "a\u0001b").to_xml` is `"<t>ab</t>"`.
- A comment containing `--` or ending with `-` raises, and so does a
  processing instruction whose target is `xml`.
- One element holds at most `SOLI_MAX_XML_ATTRS` attributes, and a tree at most
  `SOLI_MAX_XML_NODES` nodes.

## Changing a tree

```soli
list = Xml.parse("<list><item id=\"1\">old</item><item id=\"2\"/><draft/></list>")
entry = list.find("item")
entry["id"] = 10                  # set an attribute (written with .to_s)
entry["lang"] = "en"
entry.text = "new & shiny"        # replace every child with one text node
entry.name = "entry"              # rename (the name is validated)
list.root.to_xml   # => "<list><entry id=\"10\" lang=\"en\">new &amp; shiny</entry><item id=\"2\"/><draft/></list>"

entry.remove_attribute("lang")    # => "en" (nil when it was not there)
entry["id"] = nil                 # nil removes an attribute

draft = list.find("draft").remove # detach; the node stays usable
second = list.find("item[@id='2']")
second.insert_before(Xml.element("sep"))
second.insert_after("tail text")  # a String becomes a text node
list.root.prepend(Xml.element("head"))
list.root.append(draft)
second.replace(Xml.element("swapped"))
list.root.to_xml   # => "<list><head/><entry>new &amp; shiny</entry><sep/><swapped/>tail text<draft/></list>"
```

| Member | Does |
|---|---|
| `node["name"] = value`, `node.set("name", value)` | set an attribute (String, Int, Float, Bool or Decimal); `nil` removes it |
| `remove_attribute(name)` | remove it and return its value, or `nil` |
| `node.text = s` | replace all children with one text node (refused on a document) |
| `node.name = s` | rename, after checking the name |
| `append(x)`, `prepend(x)` | add `x` (an `XmlNode`, or a String for a text node) as the last or first child |
| `insert_before(x)`, `insert_after(x)`, `replace(x)` | the same, as a sibling |
| `remove` | detach the node and return it: it becomes the root of its own fragment |
| `dup` | a deep copy in a fresh tree |

These return the node they inserted or affected.

- A node from the **same** tree is **moved**. Moving a node into itself or
  into one of its descendants raises `would create a cycle`.
- A node from **another** tree is **copied**, and the copy is returned: the
  original stays where it was.
- `node.text=` and `node.name=` are the only writable properties. Any other
  dot write (`node.foo = 1`) raises `XmlNode has no writable property 'foo'`,
  and `soli check` refuses it too.
- A handle to any node keeps its whole tree alive, and a removed node stays in
  its tree (counted by `SOLI_MAX_XML_NODES`) until the tree is freed. To keep a
  small part of a large document, `dup` it: the copy is a compact tree of its
  own.
- A parsed `<a></a>` keeps its expanded form even after its children move out.

## Serialisation

`node.to_xml(options = nil)`, and `to_s`, which is the same thing.

| Option | Default | Effect |
|---|---|---|
| `indent` | `nil` (compact) | An Int (above 64 counts as 64) indents elements whose content is all elements. Mixed content is never touched. |
| `declaration` | on a document: `true` if it was parsed with one or made by `Xml.document` / `Xml.build`; on a node: `false` | Write `<?xml version="1.0" encoding="UTF-8"?>`. |
| `expand_empty` | `false` | Write `<a></a>` instead of `<a/>`. |

```soli
source = "<a x=\"1\"><b/><c></c><!--k--><?p d?></a>"
Xml.parse(source).to_xml == source                  # => true
Xml.parse("<a/>").to_xml(expand_empty: true)        # => "<a></a>"
Xml.document("r").to_xml(declaration: false)        # => "<r/>"
Xml.parse("<r xmlns:p=\"urn:p\"><p:a/></r>").root.children[0].to_xml   # => "<p:a xmlns:p=\"urn:p\"/>"
```

- With no options the output is **lossless**: an element keeps the
  self-closing form it was parsed with, attributes and namespace declarations
  keep their order and prefixes, and values are always written between `"`.
  Built empty elements are written `<a/>`.
- An element's `to_xml` is a well-formed fragment on its own: the namespace
  declarations it inherits and uses are written on its start tag.
- Escaping: text escapes `& < >` (and CR as `&#13;`); attribute values escape
  `& < > "` plus tab, LF and CR as character references, so they survive a
  re-parse.
- The output is capped at `SOLI_MAX_XML_BYTES`.

`print(node)`, `"#{node}"` and `node.inspect` all show the compact `to_xml`.
`json_stringify` and `render_json` write a node as its XML string, as they do
for a `DateTime`: use `to_h` for structured JSON.

## to_h

`to_h` is a lossy but convenient Hash view, handy with `permit` or
`render_json`:

```soli
Xml.parse("<item id=\"1\"><title>A &amp; B</title><tag>x</tag><tag>y</tag><empty/></item>").to_h
# => {"item": {"@id": "1", "title": "A & B", "tag": ["x", "y"], "empty": nil}}

Xml.parse("<a x=\"1\">t<b/></a>").to_h
# => {"a": {"@x": "1", "b": nil, "_text": "t"}}

row = Xml.parse("<product sku=\"a1\"><name>Lamp</name><price>9.5</price></product>").to_h["product"]
permit(row, {"name": true, "price": true})   # => {"name": "Lamp", "price": "9.5"}
```

- `element.to_h` is `{name => value}`; `doc.to_h` is `doc.root.to_h`.
- An element with no attributes and no element children is its text,
  trimmed, or `nil` when empty.
- Otherwise its value is a Hash: attributes under `"@name"`, children by name
  (a repeated name becomes an Array), and non-blank text under `"_text"`.
- Every value is a String: `"9.5"` stays a String.
- Nesting deeper than 1024 levels is refused.

`to_h` differs from `SOAP.parse`, which drops attributes. See
[SOAP](/docs/builtins/soap).

## Streaming large files

`Xml.each` and `Xml.reader` read a file one element at a time, so a file of
any size is handled in a bounded amount of memory (200 MiB streamed with under
2 MiB of extra memory).

```soli
total = Xml.each("exports/catalog.xml", "product[@status='active']") do |product|
  print(product.find("name").text)
end
# total is the number of matches

reader = Xml.reader("exports/catalog.xml", "product")
found = nil
while (product = reader.next)
  if product["sku"] == "b2"
    found = product
    break
  end
end
reader.close   # optional: the file is closed when the reader is dropped
```

| Call | Returns |
|---|---|
| `Xml.each(path, match, ns = nil, opts = nil) do \|node\| … end` | the number of matches yielded |
| `Xml.reader(path, match, ns = nil, opts = nil)` | an `XmlReader`: `next` (the next match, or `nil` at the end), `count` (matches returned so far), `close` |

The one option is `doctype: "skip"`, as for [`Xml.parse`](#doctype). Named
arguments work without a namespace table — a third argument that is a Hash
with a `doctype` key is read as the options:

```soli
Xml.each("imports/qualys.xml", "HOST", doctype: "skip") do |host|
  print(host.find("IP").text)
end
reader = Xml.reader("imports/report.xml", "r:item", {"r": "urn:r"}, doctype: "skip")
```

- **Files only.** For a String already in memory, use
  `Xml.parse(s).find_all(…)`. The path goes through the same checks as
  `Xml.parse_file` (the application-root jail under `soli serve` and in jobs,
  no symbolic links, no FIFOs or devices).
- **Each match is a detached `XmlNode`** carrying the namespace declarations
  in scope where it stood, so `find`, `xpath`, `to_h` and `to_xml` work on it
  alone. Its `parent` and `document` are `nil`.
- **Only the outermost match is yielded.** A match nested inside another is
  part of the outer one's subtree.
- **`break` inside an `Xml.each` block acts like `next`**: the loop goes on
  to the following match. This is the same for every block a built-in method
  runs (card #651). To stop early, use `Xml.reader`, where `break` in the
  `while` loop really stops. A `throw` inside the block propagates and closes
  the file.
- The request deadline is checked before each match.
- An error in the file raises when the reader reaches it, with a line and
  column, even after the last match:
  `Xml.each: line 3, column 9: expected </item>, found </r>`. An `XmlReader`
  raises it once, then returns `nil`.

The match is a smaller language than a query, because the reader keeps no
state beyond the chain of open elements:

- steps `name`, `p:name`, `*:name`, `*` and `p:*`, joined by `/`
  (`catalog/product` matches a `product` whose parent is a `catalog`,
  anywhere), with an optional leading `/` (from the root) or `//`;
- one predicate, on the last step only: `[@a]`, `[@a='v']` or `[@a!='v']`.

Anything else raises
`Xml.each: unsupported match at column 9: only [@a], [@a='v'] and [@a!='v'] are supported while streaming`.
Unprefixed names match in any namespace, as with `find`; `{"": uri}` in `ns`
pins them to one, and prefixes resolve through `ns`, then the root's
declarations.

`SOLI_MAX_XML_BYTES` and `SOLI_MAX_XML_NODES` cap **one match**, and one text
run or attribute value (checked before it is buffered). `SOLI_MAX_XML_DEPTH`
applies to the whole file. The file itself has no size cap.

## Errors

Errors are ordinary runtime errors, Strings with a stable prefix
(`Xml.parse:`, `Xml.parse_file:`, `Xml.find:`, `Xml.xpath:`, `Xml.each:`,
`Xml.reader:`, `Xml.element:`, `XmlNode.add:`…) and, for input errors, a line
and column:

```soli
broken = "<a><b></a>"
message = nil
try
  Xml.parse(broken)
catch error
  message = error
end
# message: "Xml.parse: line 1, column 7: expected </b>, found </a>"

fallback = Xml.parse("<oops") rescue nil   # => nil
```

Some messages:

- `Xml.parse: line 1, column 1: DOCTYPE declarations are not allowed in XML documents`
- `Xml.parse: line 1, column 4: unknown entity '&nbsp;'`
- `Xml.parse: line 1, column 5: second root element <b>`
- `Xml.parse: line 1, column 7: more than 2 nested elements (SOLI_MAX_XML_DEPTH)`
- `Xml.xpath: unbound prefix 'q'`
- `XmlNode.append: would create a cycle`

`Xml.well_formed?` answers without raising.

## Equality

`==` is identity: two handles are equal when they point at the same node of
the same tree. So `a.parent == b.parent`, `includes?` and `uniq` work on
nodes. It is not structural: `Xml.parse(s) == Xml.parse(s)` is `false`;
compare `to_xml` instead. A node is always truthy.

## Limits

Every limit can be changed with an environment variable. Parse options can
only lower them.

| Variable | Default | Caps |
|---|---|---|
| `SOLI_MAX_XML_BYTES` | `67108864` (64 MiB) | the input of `Xml.parse` and `Xml.parse_file`, one streamed match, one streamed text run or attribute, and the output of `to_xml` |
| `SOLI_MAX_XML_DEPTH` | `256` | element nesting (the parser never goes past 10,000 whatever the value) |
| `SOLI_MAX_XML_NODES` | `1000000` | nodes in one tree, removed and built nodes included |
| `SOLI_MAX_XML_ATTRS` | `1024` | attributes on one element |
| `SOLI_MAX_XPATH_VISITS` | `10000000` | node visits in one query |

Fixed limits: a name is at most 4096 bytes; an element declares at most 256
namespaces, and at most 4096 are in scope at once; an XPath expression is at
most 4096 bytes and 64 steps; `to_xml(indent:)` indents by at most 64 spaces; `to_h` nests at
most 1024 levels.

## soli check

`soli check` knows the four classes. Annotate with `XmlNode` and
`XmlDocument`:

```soli
def item_title(item: XmlNode) -> String
  item.find("name").text
end

def skus(doc: XmlDocument) -> Array
  doc.find_all("p").map { |product| product["sku"] }
end
```

- A **literal** query is compiled when the file is checked: an invalid or
  unsupported expression is a check error, and the checker knows whether the
  result is a node or a String (`find("item/@id")` is a String). A query built
  at run time is typed as unknown.
- Methods that may return `nil` (`find`, `parent`, `root`, `node["id"]`) are
  typed as `XmlNode` or `String`; the checker has no nil type, so test for
  `nil` yourself.
- Only `node.text = …` and `node.name = …` may be assigned; any other property
  write on a node is a check error.
- A node may be the condition of an `if`, `unless`, `while` or ternary, so
  `while (item = reader.next)` checks. This holds for every class-typed value,
  not only XML.
- Options, attribute Hashes, namespace tables and block parameters are typed
  as unknown.
- Avoid naming a variable after a built-in function: `lines = doc.xpath(…)`
  is read as a new value for the built-in `lines` and refused
  (`Type mismatch: expected (Any) -> Any[], found XmlNode[]`). Call it
  `line_items`.

## Security

- **No DTDs, no entity expansion.** A `DOCTYPE` is refused by default.
  `doctype: "skip"` reads past it without interpreting it: nothing is fetched
  and no entity is ever declared, so external entities (XXE) and
  entity-expansion bombs ("billion laughs") cannot happen either way.
- **Bounded work.** Depth, node count, input size, attributes per element,
  namespace declarations and XPath work are all capped (see [Limits](#limits)),
  and every error names the limit it hit.
- **No path from a String.** `Xml.parse` never reads a file. `Xml.parse_file`,
  `Xml.each` and `Xml.reader` take a path and apply the same jail as the
  `File` functions: inside the application root under `soli serve` and in
  jobs, no symbolic links, no FIFOs or devices.
- **Output is always well-formed.** The builder validates names, escapes text
  and values, drops characters XML cannot carry, and splits `]]>` in CDATA.
  Building XML by string interpolation is how injection happens; prefer the
  builder, or at least `Xml.escape`.
- **Retention.** A node keeps its whole document in memory. Keep `dup`s of the
  parts you need from a large document, not handles into it.
- **UTF-8 only.** A document in another encoding raises; convert it first.

## SOAP and XML signatures

`SOAP.parse` and `SOAP.call` read XML with this same parser, with their own
limits (nesting depth 64, 1 MiB of text per element) and their own lenient
rules, and return a Hash. See [SOAP](/docs/builtins/soap) for the shape and
for what changed when they moved to this parser.

`Xml.c14n_exclusive`, `Xml.get_element_by_id` and `Xml.get_elements_by_tag`
(exclusive canonicalisation for XML-DSig and SAML) are documented under
[XML Signatures](/docs/builtins/xml-signatures).
