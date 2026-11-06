# XML: build an RSS 2.0 feed, parse it back and query it.
#
#   soli examples/xml_feed.sl
#
# See www/docs/xml.md (/docs/builtins/xml) for the whole API.

const DC = "http://purl.org/dc/elements/1.1/"

posts = [
  {"id": 1, "title": "XML lands in Soli", "author": "Ada", "url": "https://example.com/posts/xml",
   "body_html": "<p>Parse, query, build and stream.</p>", "tags": ["xml", "release"]},
  {"id": 2, "title": "XPath, the subset", "author": "Grace", "url": "https://example.com/posts/xpath",
   "body_html": "<p>Predicates & unions.</p>", "tags": ["xml"]},
  {"id": 3, "title": "Tom & Jerry's feed", "author": "Ada", "url": "https://example.com/posts/feeds",
   "body_html": "<p>Escaping is the builder's job.</p>", "tags": []}
]

# --- Build -----------------------------------------------------------------
# Xml.build gives the block the root element and returns the document.
# node.add(name, content, attrs) returns the child; named arguments are
# attributes, and a block receives the child.
feed = Xml.build("rss", {"version": "2.0", "xmlns:dc": DC}) do |rss|
  rss.add("channel") do |channel|
    channel.add("title", "Soli blog")
    channel.add("link", "https://example.com")
    channel.add("description", "News & releases")
    posts.each do |post|
      channel.add("item", id: post.id) do |item|
        item.add("title", post.title)
        item.add("link", post.url)
        item.add("dc:creator", post.author)
        post.tags.each do |tag|
          item.add("category", tag)
        end
        item.add("description").add_cdata(post.body_html)
      end
    end
  end
end

xml = feed.to_xml(indent: 2)
print(xml)

# --- Parse it back ---------------------------------------------------------
doc = Xml.parse(xml)
channel = doc.find("channel")
print("Channel: #{channel.find("title").text} (#{channel.find_all("item").length} items)")

# find / find_all: relative paths start at the root element, and an
# unprefixed name matches in any namespace.
doc.find_all("channel/item").each do |item|
  categories = item.find_all("category/text()").join(", ")
  print("  ##{item["id"]} #{item.find("title").text} by #{item.find("creator").text} [#{categories}]")
end

# xpath / at_xpath: strict XPath 1.0. The dc: prefix resolves through the
# declarations on the root element.
by_ada = doc.xpath("//item[dc:creator='Ada']/title/text()")
print("By Ada: #{by_ada.join(" | ")}")
print("Tagged release: #{doc.at_xpath("//item[category='release']/@id")}")
print("Last item: #{doc.at_xpath("/rss/channel/item[last()]/link/text()")}")
print("Untagged: #{doc.find_all("channel/item[not(category)]/@id")}")

# The text of a node is decoded: the CDATA comes back as written.
print("First body: #{doc.find("channel/item/description").text}")

# --- Change it -------------------------------------------------------------
doc.find("channel/item[@id='3']").remove
channel.find("title").text = "Soli blog (archive)"
channel.add("lastBuildDate", "Fri, 06 Nov 2026 09:00:00 GMT")
print("After edits: #{doc.find_all("channel/item/@id")}")

# --- A Hash view, for permit or JSON ----------------------------------------
first_item = doc.find("channel/item").to_h["item"]
print(permit(first_item, {"title": true, "link": true}))
