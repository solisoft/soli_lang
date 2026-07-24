import UIKit
import UserNotifications
import WebKit

@main
class AppDelegate: UIResponder, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    var window: UIWindow?
    static let startURL = URL(string: "{{START_URL}}")!
    static let scheme = "{{SCHEME}}"
    var pendingOpen: URL?
    var apnsTokenHex: String?

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound]) { _, _ in
            DispatchQueue.main.async { application.registerForRemoteNotifications() }
        }
        let window = UIWindow(frame: UIScreen.main.bounds)
        let nav = WebViewController()
        window.rootViewController = nav
        window.makeKeyAndVisible()
        self.window = window
        if let url = launchOptions?[.url] as? URL {
            pendingOpen = url
        }
        return true
    }

    func application(_ app: UIApplication, open url: URL,
                     options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
        deliver(url)
        return true
    }

    func application(_ application: UIApplication,
                     continue userActivity: NSUserActivity,
                     restorationHandler: @escaping ([UIUserActivityRestoring]?) -> Void) -> Bool {
        if userActivity.activityType == NSUserActivityTypeBrowsingWeb,
           let url = userActivity.webpageURL {
            deliver(url)
            return true
        }
        return false
    }

    func application(_ application: UIApplication,
                     didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
        apnsTokenHex = hex
        postDeviceToken(hex)
    }

    func deliver(_ url: URL) {
        if let nav = window?.rootViewController as? WebViewController {
            nav.open(url)
        } else {
            pendingOpen = url
        }
    }

    func postDeviceToken(_ token: String) {
        guard let web = (window?.rootViewController as? WebViewController)?.webView else { return }
        web.configuration.websiteDataStore.httpCookieStore.getAllCookies { cookies in
            let relevant = cookies.filter { cookie in
                AppDelegate.startURL.host.map { cookie.domain.contains($0) } ?? false
            }
            let header = relevant.map { "\($0.name)=\($0.value)" }.joined(separator: "; ")
            if header.isEmpty { return }
            let base = AppDelegate.startURL.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            guard let endpoint = URL(string: base + "/devices") else { return }
            var req = URLRequest(url: endpoint)
            req.httpMethod = "POST"
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.setValue(header, forHTTPHeaderField: "Cookie")
            // Origin helps the same-origin CSRF gate; routes also skip_csrf /devices/*.
            req.setValue(base, forHTTPHeaderField: "Origin")
            req.setValue(base + "/", forHTTPHeaderField: "Referer")
            let body: [String: String] = ["platform": "ios", "token": token]
            req.httpBody = try? JSONSerialization.data(withJSONObject: body)
            URLSession.shared.dataTask(with: req).resume()
        }
    }
}
