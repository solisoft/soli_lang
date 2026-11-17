# The document /pdf renders: a one-page pass naming the datacenter that built
# it. Not a database model: a layout template and the data it is filled with,
# kept in app/models so it loads before the controllers.
class EdgePass
  const MAX_NAME_LENGTH = 40

  # The data the template interpolates, from the request that asks for it.
  static def data(runtime, name)
    holder = name.to_s.trim
    holder = "Anonymous visitor" if holder.blank?
    holder = holder.substring(0, MAX_NAME_LENGTH) if holder.length > MAX_NAME_LENGTH
    {
      "holder": holder,
      "colo": runtime["colo"],
      "country": runtime["country"],
      "rendered_at": runtime["rendered_at"],
      "runtime": runtime["runtime"],
      "version": runtime["version"],
      "facts": [
        {"label": "Datacenter", "value": runtime["colo"]},
        {"label": "Country", "value": runtime["country"]},
        {"label": "Rendered at (UTC)", "value": runtime["rendered_at"]},
        {"label": "Runtime", "value": runtime["runtime"]},
        {"label": "Soli", "value": runtime["version"]}
      ]
    }
  end

  # The layout, as the JSON `pdf_render` takes. No font folder: on the edge the
  # runtime falls back to the Titillium Web faces compiled into it.
  static def template
    {
      "fonts": ["titillium"],
      "options": {"margins": 56},
      "content": [
        {"type": "box", "fill": "0F172A", "radius": 6, "padding": 22, "gap": 24, "content": [
          {"type": "paragraph", "value": "EDGE PASS",
            "options": {"fontSize": 11, "fontWeight": "bold", "color": "F6821F"}},
          {"type": "paragraph", "value": "${holder}",
            "options": {"fontSize": 30, "fontWeight": "bold", "color": "FFFFFF", "spacing": 4}},
          {"type": "paragraph", "value": "Rendered in ${colo} by a Cloudflare Worker",
            "options": {"fontSize": 12, "color": "CBD5E1"}}
        ]},
        {"type": "paragraph",
          "value": "This PDF did not exist before you asked for it. A Soli controller built it inside " +
            "the Worker that answered your request: the layout engine, the font and the PDF writer " +
            "are part of the WebAssembly runtime, so no other server took part.",
          "options": {"fontSize": 11, "lineHeight": 1.5, "spacing": 18}},
        {"type": "table", "data": "facts",
          "rows": [[
            {"text": "${label}", "width": 170, "fontSize": 10, "color": "64748B"},
            {"text": "${value}", "width": 300, "fontSize": 11, "fontWeight": "bold"}
          ]],
          "options": {"stripe": "F1F5F9", "padding_x": 8, "padding_y": 7}},
        {"type": "move", "x": 0, "y": 22},
        {"type": "qr", "kind": "text", "value": "https://cf.solisoft.net/pdf", "width": 84},
        {"type": "move", "x": 0, "y": 96},
        {"type": "paragraph", "value": "cf.solisoft.net/pdf",
          "options": {"fontSize": 9, "color": "64748B", "link": "https://cf.solisoft.net/pdf"}}
      ],
      "footer": [
        {"type": "hr"},
        {"type": "paragraph", "value": "Soli ${version} on ${runtime}  ·  page #PAGE# of #PAGES#",
          "options": {"fontSize": 8, "color": "94A3B8", "alignment": "center"}}
      ]
    }
  end
end
