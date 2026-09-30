# frozen_string_literal: true

require_relative 'pdf/version'
require_relative 'pdf/errors'
require_relative 'pdf/binary'
require_relative 'pdf/renderer'

module Soli
  # Generate PDFs from JSON with the Soli renderer.
  #
  #   pdf = Soli::PDF.render(template: template_hash, data: { "invoice" => { "number" => "F-42" } })
  #   File.binwrite("invoice.pdf", pdf)
  module PDF
    class << self
      # Renders a PDF and returns its bytes (a binary String).
      #
      # template: - layout, Hash/Array or JSON String
      # data:     - data document (Hash/Array/String); or invoice: - a typed invoice
      # xml:      - Factur-X CII XML to embed (with data:)
      # options:  title, author, subject, profile, password, owner_password, stationery (path),
      #           attachments (paths), fonts (extra font dirs), images: false
      def render(template:, data: nil, invoice: nil, xml: nil, **options)
        renderer = Renderer.new(template: template, data: data, invoice: invoice, xml: xml, **options)
        renderer.call.tap { @last_warnings = renderer.warnings }
      end

      # Same as render, writing to `path`; returns the path.
      def render_to_file(path, **args)
        File.binwrite(path, render(**args))
        path
      end

      # Warnings (skipped images, missing glyphs…) of the last successful render.
      def last_warnings
        @last_warnings || []
      end

      def binary_path=(path)
        Binary.path = path
      end
    end
  end
end
