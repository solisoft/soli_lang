# frozen_string_literal: true

require_relative 'lib/soli/pdf/version'

Gem::Specification.new do |spec|
  spec.name = 'soli-pdf'
  spec.version = Soli::PDF::VERSION
  spec.authors = ['Olivier Bonnaure']
  spec.email = ['olivier@solisoft.net']

  spec.summary = 'Generate PDFs from JSON with the Soli renderer.'
  spec.description = <<~TEXT
    Wraps the standalone `render_pdf` binary of Soli: give it a JSON layout
    template and a JSON data document, get a PDF back. The binary is
    downloaded from the GitHub release on first use, checked against its
    published SHA-256 and cached.
  TEXT
  spec.homepage = 'https://github.com/solisoft/soli_lang'
  spec.license = 'MIT'
  spec.required_ruby_version = '>= 3.2'

  spec.metadata['source_code_uri'] = "#{spec.homepage}/tree/main/gem/soli-pdf"
  spec.metadata['changelog_uri'] = "#{spec.homepage}/blob/main/gem/soli-pdf/CHANGELOG.md"
  spec.metadata['rubygems_mfa_required'] = 'true'

  spec.files = Dir['lib/**/*.rb', 'README.md', 'LICENSE', 'CHANGELOG.md']
  spec.require_paths = ['lib']
end
