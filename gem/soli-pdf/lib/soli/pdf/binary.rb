# frozen_string_literal: true

require 'digest'
require 'fileutils'
require 'net/http'
require 'rbconfig'
require 'tmpdir'
require 'uri'

module Soli
  module PDF
    # Finds the render_pdf executable, downloading the release tarball on first use.
    class Binary
      RELEASE_URL = 'https://github.com/solisoft/soli_lang/releases/download'
      MAX_REDIRECTS = 5

      class << self
        attr_writer :path

        # Absolute path of the executable.
        def path
          configured = @path || ENV.fetch('SOLI_PDF_BIN', nil)
          return check(configured) if configured

          installed = File.join(install_dir, executable_name)
          download unless File.executable?(installed)
          installed
        end

        # Directory holding the bundled fonts next to the executable, if any.
        def fonts_dir
          dir = File.join(File.dirname(path), 'fonts')
          dir if File.directory?(dir)
        end

        def artifact
          "render-pdf-#{os}-#{arch}"
        end

        def executable_name
          os == 'windows' ? 'render_pdf.exe' : 'render_pdf'
        end

        def install_dir
          cache = ENV['XDG_CACHE_HOME'] || File.join(Dir.home, '.cache')
          File.join(cache, 'soli-pdf', BINARY_VERSION, artifact)
        end

        private

        def check(configured)
          File.executable?(configured) ? File.expand_path(configured) : raise(BinaryNotFound, "not executable: #{configured}")
        end

        def os
          case RbConfig::CONFIG['host_os']
          when /linux/ then 'linux'
          when /darwin/ then 'darwin'
          when /mswin|mingw|cygwin/ then 'windows'
          else raise BinaryNotFound, "unsupported OS: #{RbConfig::CONFIG['host_os']}"
          end
        end

        def arch
          case RbConfig::CONFIG['host_cpu']
          when /x86_64|amd64|x64/ then 'amd64'
          when /aarch64|arm64/ then 'arm64'
          else raise BinaryNotFound, "unsupported CPU: #{RbConfig::CONFIG['host_cpu']}"
          end
        end

        def download
          base = "#{RELEASE_URL}/v#{BINARY_VERSION}/#{artifact}.tar.gz"
          tarball = fetch(base)
          expected = fetch("#{base}.sha256").split.first.to_s.downcase
          actual = Digest::SHA256.hexdigest(tarball)
          raise DownloadError, "SHA-256 mismatch for #{base}: expected #{expected}, got #{actual}" unless actual == expected

          extract(tarball)
        end

        # Unpack into a sibling temp dir, then rename: a half-extracted install is never visible.
        def extract(tarball)
          FileUtils.mkdir_p(File.dirname(install_dir))
          Dir.mktmpdir('soli-pdf', File.dirname(install_dir)) do |staging|
            archive = File.join(staging, 'asset.tar.gz')
            File.binwrite(archive, tarball)
            unpacked = File.join(staging, 'out')
            FileUtils.mkdir_p(unpacked)
            system('tar', '-xzf', archive, '-C', unpacked) || raise(DownloadError, 'could not extract the release tarball')
            binary = File.join(unpacked, executable_name)
            raise DownloadError, "#{executable_name} missing from the release tarball" unless File.file?(binary)

            FileUtils.chmod(0o755, binary)
            FileUtils.rm_rf(install_dir)
            FileUtils.mv(unpacked, install_dir)
          end
        end

        def fetch(url, redirects = MAX_REDIRECTS)
          uri = URI(url)
          response = Net::HTTP.start(uri.host, uri.port, use_ssl: true, open_timeout: 10, read_timeout: 120) do |http|
            http.request(Net::HTTP::Get.new(uri))
          end
          case response
          when Net::HTTPSuccess then response.body
          when Net::HTTPRedirection
            raise DownloadError, "too many redirects fetching #{url}" if redirects.zero?

            fetch(response['location'], redirects - 1)
          else raise DownloadError, "#{url}: HTTP #{response.code}"
          end
        rescue SocketError, SystemCallError, Timeout::Error, OpenSSL::SSL::SSLError => e
          raise DownloadError, "#{url}: #{e.message}"
        end
      end
    end
  end
end
