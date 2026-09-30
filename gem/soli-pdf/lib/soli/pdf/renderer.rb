# frozen_string_literal: true

require 'json'
require 'open3'
require 'tempfile'

module Soli
  module PDF
    # Builds the render_pdf command line and runs it. Hash/Array inputs are
    # serialised to JSON; Strings are passed through as JSON text.
    class Renderer
      # option => flag for the plain `--flag value` options.
      VALUE_FLAGS = {
        title: '--title', author: '--author', subject: '--subject', profile: '--profile',
        password: '--password', owner_password: '--owner-password'
      }.freeze

      def initialize(template:, data: nil, invoice: nil, xml: nil, **options)
        raise ArgumentError, 'give either data: or invoice:' if data.nil? == invoice.nil?
        raise ArgumentError, 'xml: cannot be combined with invoice:' if invoice && xml

        @template = template
        @data = data
        @invoice = invoice
        @xml = xml
        @options = options
        unknown = options.keys - VALUE_FLAGS.keys - %i[fonts images stationery attachments]
        raise ArgumentError, "unknown option(s): #{unknown.join(', ')}" unless unknown.empty?
      end

      # PDF bytes (binary String). Warnings printed by render_pdf land in `warnings`.
      def call
        files = []
        argv = [Binary.path]

        # The template goes through a temp file; the data (or invoice) travels on stdin.
        template_file = tempfile(json(@template))
        files << template_file
        argv.push('--template', template_file.path)
        stdin_payload = json(@data.nil? ? @invoice : @data)
        argv.push(@data.nil? ? '--invoice' : '--data', '-')
        argv.push('-o', '-')
        add_options(argv, files)

        stdout, stderr, status = Open3.capture3(*argv, stdin_data: stdin_payload, binmode: true)
        raise RenderError.new(status, stderr) unless status.success?

        @warnings = stderr.lines.grep(/\Awarning:/).map { |line| line.sub(/\Awarning:\s*/, '').strip }
        stdout
      ensure
        files&.each(&:close!)
      end

      def warnings
        @warnings || []
      end

      private

      def add_options(argv, files)
        VALUE_FLAGS.each { |key, flag| argv.push(flag, @options[key].to_s) unless @options[key].nil? }
        argv.push('--no-images') if @options[:images] == false
        argv.push('--stationery', @options[:stationery].to_s) if @options[:stationery]
        Array(@options[:attachments]).each { |path| argv.push('--attach', path.to_s) }
        if @xml
          file = tempfile(@xml.to_s)
          files << file
          argv.push('--xml', file.path)
        end
        fonts = Array(@options[:fonts])
        fonts += [Binary.fonts_dir] if Binary.fonts_dir
        fonts.each { |dir| argv.push('--font-dir', dir.to_s) }
      end

      def json(value)
        value.is_a?(String) ? value : JSON.generate(value)
      end

      def tempfile(content)
        file = Tempfile.new(['soli-pdf', '.json'], binmode: true)
        file.write(content)
        file.flush
        file
      end
    end
  end
end
