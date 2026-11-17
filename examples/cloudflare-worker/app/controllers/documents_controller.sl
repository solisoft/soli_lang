# Twice the width the page shows it at, for sharp text on a dense screen.
const PREVIEW_WIDTH = 720

class DocumentsController < Controller
  # GET /pdf — a PDF rendered inside the Worker, and how
  def index(req)
    @title = "PDF"
    @description = "A PDF rendered inside a Cloudflare Worker by a Soli controller: the layout engine and its " +
      "font are part of the WebAssembly runtime, built with scripts/build-edge.sh --pdf."
    @runtime = RuntimeInfo.of(req)
    @name = params["name"].to_s
    @preview_url = "/pdf/pass.png?name=" + url_encode(@name)
  end

  # GET /pdf/pass.pdf — the pass itself, for ?name=
  def pass(req)
    data = EdgePass.data(RuntimeInfo.of(req), params["name"])
    pdf_response(EdgePass.template.to_json, data.to_json, {"title": "Edge pass for #{data["holder"]}"})
  end

  # GET /pdf/pass.png — the same page as an image, for ?name=
  def preview(req)
    data = EdgePass.data(RuntimeInfo.of(req), params["name"])
    pdf_preview_response(EdgePass.template.to_json, data.to_json, {"width": PREVIEW_WIDTH})
  end
end
