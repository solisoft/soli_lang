# frozen_string_literal: true

module Soli
  module PDF
    class Error < StandardError; end

    # No usable render_pdf binary: unsupported platform, or a configured path that does not exist.
    class BinaryNotFound < Error; end

    # The release asset could not be downloaded, or did not match its SHA-256.
    class DownloadError < Error; end

    # render_pdf exited non-zero. `stderr` holds its message.
    class RenderError < Error
      attr_reader :status, :stderr

      def initialize(status, stderr)
        @status = status
        @stderr = stderr
        super(stderr.to_s.strip.empty? ? "render_pdf failed (#{status})" : stderr.to_s.strip)
      end
    end
  end
end
