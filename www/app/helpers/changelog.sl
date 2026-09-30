# The docs changelog, a page at a time.
#
# `docs/getting-started/changelog.html.slv` stays one hand-authored file, newest
# first, a `<section id="vX-Y-Z">` per release (see the repo CLAUDE.md). It had
# grown past 600 KB, so `changelog_paged.html.slv` renders it to a string and
# these helpers cut it at those section markers: the intro and the Unreleased
# block, then one page of releases, then whatever follows the last release (the
# screenshot lightbox script). Nothing about writing an entry changes.

# Helper files contribute their functions only (their top level is not run),
# so the constants are functions too.
def changelog_per_page() -> Int
  8
end

def changelog_release_marker() -> String
  "    <section id=\"v"
end

def changelog_section_end() -> String
  "    </section>\n"
end

# The rendered page split into its parts:
#   { "intro", "unreleased", "releases": [{ "id", "label", "html" }], "tail" }
def changelog_parts(full_html: String) -> Hash
  head_and_releases = full_html.split(changelog_release_marker())
  page_head = head_and_releases[0]
  chunks = head_and_releases.slice(1, head_and_releases.length)

  # The last release carries the page's tail after its closing tag.
  last_pieces = chunks[chunks.length - 1].split(changelog_section_end())
  tail = last_pieces[last_pieces.length - 1]
  chunks[chunks.length - 1] = last_pieces.slice(0, last_pieces.length - 1).join(changelog_section_end()) + changelog_section_end()

  releases = chunks.map do |chunk|
    id = "v" + chunk.split("\"")[0]
    { "id": id, "label": id.replace("-", "."), "html": changelog_release_marker() + chunk }
  end

  unreleased_marker = "    <section id=\"unreleased\""
  head_pieces = page_head.split(unreleased_marker)
  unreleased = ""
  unreleased = unreleased_marker + head_pieces.slice(1, head_pieces.length).join(unreleased_marker) if head_pieces.length > 1

  { "intro": head_pieces[0], "unreleased": unreleased, "releases": releases, "tail": tail }
end

# Page `requested` (1-based, clamped) of the changelog, pager included.
def changelog_page_html(full_html: String, requested: Int) -> String
  parts = changelog_parts(full_html)
  releases = parts["releases"]
  total = (releases.length + changelog_per_page() - 1) / changelog_per_page()
  page = requested
  page = 1 if page < 1
  page = total if page > total

  first = (page - 1) * changelog_per_page()
  last = first + changelog_per_page()
  last = releases.length if last > releases.length
  shown = releases.slice(first, last).map { |release| release["html"] }.join("")

  pager = changelog_pager(releases, page, total)
  body = parts["intro"]
  body = body + parts["unreleased"] if page == 1
  body + pager + shown + pager + changelog_anchor_script(releases, page) + parts["tail"]
end

# Newer / Older, and a link per page named by the releases it holds.
def changelog_pager(releases: Array, page: Int, total: Int) -> String
  links = (1..total + 1).map do |number|
    first = releases[(number - 1) * changelog_per_page()]["label"]
    last_index = number * changelog_per_page() - 1
    last_index = releases.length - 1 if last_index > releases.length - 1
    span_label = first + " – " + releases[last_index]["label"]
    if number == page
      "<span class=\"cl-page is-current\" aria-current=\"page\">" + span_label + "</span>"
    else
      "<a class=\"cl-page\" href=\"?page=" + str(number) + "\">" + span_label + "</a>"
    end
  end

  newer = "<span class=\"cl-step is-disabled\">&larr; Newer</span>"
  newer = "<a class=\"cl-step\" href=\"?page=" + str(page - 1) + "\">&larr; Newer</a>" if page > 1
  older = "<span class=\"cl-step is-disabled\">Older &rarr;</span>"
  older = "<a class=\"cl-step\" href=\"?page=" + str(page + 1) + "\">Older &rarr;</a>" if page < total

  "<nav class=\"cl-pager\" aria-label=\"Changelog pages\">" + newer +
    "<div class=\"cl-pages\">" + links.join("") + "</div>" + older + "</nav>\n"
end

# A link to a release on another page (`#v2-3-0`, from anywhere on the site)
# lands on the page that holds it.
def changelog_anchor_script(releases: Array, page: Int) -> String
  anchor_pairs = []
  releases.each_with_index do |release, index|
    anchor_pairs.push("\"" + release["id"] + "\":" + str(index / changelog_per_page() + 1))
  end
  "<script>(function () { var pages = {" + anchor_pairs.join(",") + "}; var id = location.hash.slice(1);" +
    " if (id && pages[id] && pages[id] !== " + str(page) + " && !document.getElementById(id)) {" +
    " location.replace('?page=' + pages[id] + '#' + id); } })();</script>\n"
end
