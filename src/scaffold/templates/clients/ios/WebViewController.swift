import UIKit
import WebKit
import UserNotifications
import LocalAuthentication
import CoreNFC

class WebViewController: UIViewController, WKScriptMessageHandler, WKUIDelegate, WKNavigationDelegate, NFCNDEFReaderSessionDelegate {
    var webView: WKWebView!
    private var pendingDeepLink: URL?
    private var nfcSession: NFCNDEFReaderSession?
    private var nfcReplyId: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(red: 0.05, green: 0.06, blue: 0.07, alpha: 1)

        let content = WKUserContentController()
        let bridge = """
        window.soli = window.soli || {};
        window.soli.native = {
          platform: "ios",
          capabilities: ["notify","geolocation","vibrate","share","keep_awake","print","clipboard","badge","camera","nfc","biometric"],
          notify: function(json) { window.webkit.messageHandlers.soliNative.postMessage(typeof json === "string" ? json : JSON.stringify(json)); },
          call: function(json) { window.webkit.messageHandlers.soliNative.postMessage(typeof json === "string" ? json : JSON.stringify(json)); }
        };
        """
        content.addUserScript(WKUserScript(source: bridge, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        content.add(self, name: "soliNative")

        let config = WKWebViewConfiguration()
        config.userContentController = content
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []

        webView = WKWebView(frame: view.bounds, configuration: config)
        webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        webView.uiDelegate = self
        webView.navigationDelegate = self
        view.addSubview(webView)
        webView.load(URLRequest(url: AppDelegate.startURL))

        if let app = UIApplication.shared.delegate as? AppDelegate, let url = app.pendingOpen {
            app.pendingOpen = nil
            open(url)
        }
    }

    func open(_ url: URL) {
        if url.scheme == AppDelegate.scheme {
            var path = url.path
            if path.isEmpty { path = "/" }
            let base = AppDelegate.startURL.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            if let mapped = URL(string: base + path + (url.query.map { "?\($0)" } ?? "")) {
                if webView != nil { webView.load(URLRequest(url: mapped)) }
                else { pendingDeepLink = mapped }
            }
        } else {
            if webView != nil { webView.load(URLRequest(url: url)) }
            else { pendingDeepLink = url }
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if let pending = pendingDeepLink {
            pendingDeepLink = nil
            webView.load(URLRequest(url: pending))
        }
        if let app = UIApplication.shared.delegate as? AppDelegate, let token = app.apnsTokenHex {
            app.postDeviceToken(token)
        }
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? String,
              let data = body.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }

        if let method = obj["method"] as? String, let id = obj["id"] as? String {
            handleCall(method: method, id: id, params: obj["params"] as? [String: Any] ?? [:])
            return
        }
        let title = obj["title"] as? String ?? "{{APP_NAME}}"
        let bodyText = obj["body"] as? String ?? ""
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = bodyText
        if let url = obj["url"] as? String { content.userInfo["url"] = url }
        let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(req, withCompletionHandler: nil)
    }

    func handleCall(method: String, id: String, params: [String: Any]) {
        switch method {
        case "badge":
            let count = params["count"] as? Int ?? 0
            UIApplication.shared.applicationIconBadgeNumber = count
            reply(id: id, result: count, error: nil)
        case "vibrate":
            let gen = UIImpactFeedbackGenerator(style: .medium)
            gen.impactOccurred()
            reply(id: id, result: true, error: nil)
        case "share":
            var items: [Any] = []
            if let t = params["text"] as? String { items.append(t) }
            if let u = params["url"] as? String, let url = URL(string: u) { items.append(url) }
            let ac = UIActivityViewController(activityItems: items, applicationActivities: nil)
            present(ac, animated: true) { self.reply(id: id, result: true, error: nil) }
        case "keep_awake":
            let on = params["on"] as? Bool ?? true
            UIApplication.shared.isIdleTimerDisabled = on
            reply(id: id, result: on, error: nil)
        case "authenticate":
            let ctx = LAContext()
            let reason = params["reason"] as? String ?? "Confirm"
            ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) { ok, err in
                DispatchQueue.main.async {
                    if ok { self.reply(id: id, result: true, error: nil) }
                    else { self.reply(id: id, result: nil, error: err?.localizedDescription ?? "cancelled") }
                }
            }
        case "readTag":
            guard NFCNDEFReaderSession.readingAvailable else {
                reply(id: id, result: nil, error: "NFC unavailable")
                return
            }
            nfcReplyId = id
            nfcSession = NFCNDEFReaderSession(delegate: self, queue: nil, invalidateAfterFirstRead: true)
            nfcSession?.begin()
        case "print":
            let controller = UIPrintInteractionController.shared
            let info = UIPrintInfo.printInfo()
            info.jobName = "{{APP_NAME}}"
            controller.printInfo = info
            controller.printFormatter = webView.viewPrintFormatter()
            controller.present(animated: true) { _, _, _ in self.reply(id: id, result: true, error: nil) }
        default:
            reply(id: id, result: nil, error: "unsupported: \(method)")
        }
    }

    func reply(id: String, result: Any?, error: String?) {
        var payload: [String: Any] = ["id": id]
        if let error = error { payload["error"] = error }
        else { payload["result"] = result ?? NSNull() }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else { return }
        let quoted = json
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "'", with: "\\'")
        webView.evaluateJavaScript("window.__soliNativeReply && window.__soliNativeReply('\(quoted)')", completionHandler: nil)
    }

    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError error: Error) {
        if let id = nfcReplyId {
            reply(id: id, result: nil, error: error.localizedDescription)
            nfcReplyId = nil
        }
    }

    func readerSession(_ session: NFCNDEFReaderSession, didDetectNDEFs messages: [NFCNDEFMessage]) {
        if let id = nfcReplyId {
            reply(id: id, result: "tag", error: nil)
            nfcReplyId = nil
        }
    }
}
