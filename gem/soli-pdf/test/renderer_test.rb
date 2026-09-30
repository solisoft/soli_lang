# frozen_string_literal: true

require 'minitest/autorun'
require 'json'
require 'tmpdir'
require 'soli/pdf'

class RendererTest < Minitest::Test
  SAMPLES = File.expand_path('../../../pdf/samples', __dir__)

  def with_fake_binary(script)
    Dir.mktmpdir do |dir|
      path = File.join(dir, 'render_pdf')
      File.write(path, "#!/bin/sh\n#{script}\n")
      File.chmod(0o755, path)
      Soli::PDF::Binary.path = path
      yield dir
    ensure
      Soli::PDF::Binary.path = nil
    end
  end

  def test_passes_data_on_stdin_and_reads_pdf_from_stdout
    with_fake_binary('cat >&2; printf "%%PDF-fake"') do
      pdf = Soli::PDF.render(template: { 'content' => [] }, data: { 'a' => 1 })
      assert_equal '%PDF-fake', pdf
    end
  end

  def test_builds_flags
    with_fake_binary('echo "$@" >&2; exit 3') do
      error = assert_raises(Soli::PDF::RenderError) do
        Soli::PDF.render(template: {}, data: {}, title: 'T', images: false, fonts: ['/f'])
      end
      assert_match(/--title T/, error.message)
      assert_match(/--no-images/, error.message)
      assert_match(%r{--font-dir /f}, error.message)
      assert_match(/-o -/, error.message)
    end
  end

  def test_warnings_are_collected
    with_fake_binary('cat >/dev/null; echo "warning: image skipped" >&2; printf x') do
      Soli::PDF.render(template: {}, data: {})
      assert_equal ['image skipped'], Soli::PDF.last_warnings
    end
  end

  def test_needs_data_or_invoice
    assert_raises(ArgumentError) { Soli::PDF.render(template: {}) }
    assert_raises(ArgumentError) { Soli::PDF.render(template: {}, data: {}, invoice: {}) }
  end

  def test_unknown_option
    assert_raises(ArgumentError) { Soli::PDF.render(template: {}, data: {}, bogus: 1) }
  end

  def test_real_binary
    skip 'set SOLI_PDF_BIN to a built render_pdf' unless ENV['SOLI_PDF_BIN']

    template = JSON.parse(File.read(File.join(SAMPLES, 'letter.json')))
    data = JSON.parse(File.read(File.join(SAMPLES, 'letter_data.json')))
    pdf = Soli::PDF.render(template: template, data: data, images: false,
                           fonts: [File.expand_path('../../../pdf/fonts', __dir__)])
    assert pdf.start_with?('%PDF-')
    assert_equal Encoding::BINARY, pdf.encoding
  end
end
