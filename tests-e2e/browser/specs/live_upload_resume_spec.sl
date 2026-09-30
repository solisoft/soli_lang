# Chunked LiveView uploads that pause and resume (src/live/client.js +
# src/live/upload.rs). A real browser, because the interesting part is the
# client: which chunks it decides to send after a pause or an interruption.

# Helpers run in the page. `count_posts` wraps XMLHttpRequest so a spec can see
# how many chunks were actually sent.
const PAGE_HELPERS = [[
window.__posts = 0;
(function () {
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function (body) {
    if (this.__soliUpload) window.__posts += 1;
    return send.call(this, body);
  };
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__soliUpload = String(url).indexOf('/live/upload') === 0 && method === 'POST';
    return open.apply(this, arguments);
  };
})();
window.__pick = function (name, size) {
  const bytes = new Uint8Array(size);
  const file = new File([bytes], name, { lastModified: 1700000000000 });
  const transfer = new DataTransfer();
  transfer.items.add(file);
  const input = document.getElementById('upl');
  input.files = transfer.files;
  input.dispatchEvent(new Event('change', { bubbles: true }));
};
]]

describe("liveview upload resume", fn() {
    test("a chunked upload completes and the handler sees the whole file", fn() {
        visit("/uploads")
        wait_for("#upl")
        evaluate(PAGE_HELPERS)

        evaluate("window.__pick('big.bin', 716800)")
        wait_for_text("result=big.bin:716800")

        # 716800 bytes at 256 KiB is three chunks.
        assert_eq(evaluate("window.__posts"), 3)
    })

    test("a paused upload sends nothing until it is resumed", fn() {
        visit("/uploads")
        wait_for("#upl")
        evaluate(PAGE_HELPERS)

        click("#pause")
        assert_eq(evaluate("document.getElementById('upl').hasAttribute('data-soli-upload-paused')"), true)

        evaluate("window.__pick('paused.bin', 716800)")
        # Give the client time to send if it were going to.
        sleep(0.8)
        assert_eq(evaluate("window.__posts"), 0)

        click("#pause")
        wait_for_text("result=paused.bin:716800")
        assert_eq(evaluate("window.__posts"), 3)
    })

    test("picking the same file again resumes from the chunks the server holds", fn() {
        visit("/uploads")
        wait_for("#upl")
        evaluate(PAGE_HELPERS)

        # Play the part of an interrupted earlier attempt: two of three chunks
        # already on the server, and the id remembered under the file's
        # fingerprint, exactly as the client would have left them.
        evaluate([[
          (function () {
            const id = 'resume-test-' + Date.now();
            const key = ['soli-upload', 'file', 'partial.bin', 716800, 1700000000000].join('|');
            localStorage.setItem(key, id);
            const chunk = (i, size) => {
              const body = new FormData();
              body.append('file', new Blob([new Uint8Array(size)]), 'partial.bin');
              return fetch('/live/upload', {
                method: 'POST',
                body: body,
                credentials: 'same-origin',
                headers: { 'X-Soli-Upload-Id': id, 'X-Soli-Chunk-Index': String(i), 'X-Soli-Chunk-Count': '3' }
              });
            };
            chunk(0, 262144).then(() => chunk(1, 262144)).then(() => {
              const marker = document.createElement('i');
              marker.id = 'seeded';
              document.body.appendChild(marker);
            });
          })()
        ]])
        wait_for("#seeded")

        evaluate("window.__pick('partial.bin', 716800)")
        wait_for_text("result=partial.bin:716800")

        # Only the missing chunk crossed the wire.
        assert_eq(evaluate("window.__posts"), 1)
    })

    test("an upload the server has forgotten starts over under a new id", fn() {
        visit("/uploads")
        wait_for("#upl")
        evaluate(PAGE_HELPERS)

        evaluate("localStorage.setItem(['soli-upload', 'file', 'stale.bin', 716800, 1700000000000].join('|'), 'expired-long-ago')")
        evaluate("window.__pick('stale.bin', 716800)")
        wait_for_text("result=stale.bin:716800")

        assert_eq(evaluate("window.__posts"), 3)
    })
})
