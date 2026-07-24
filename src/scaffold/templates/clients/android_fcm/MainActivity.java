package {{JAVA_PACKAGE}};

import android.annotation.SuppressLint;
import android.os.Bundle;
import android.webkit.CookieManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import androidx.appcompat.app.AppCompatActivity;
import com.google.firebase.messaging.FirebaseMessaging;
import org.json.JSONObject;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;

public class MainActivity extends AppCompatActivity {
    static final String START_URL = "{{START_URL}}";
    private WebView webView;

    @Override
    @SuppressLint({"SetJavaScriptEnabled", "AddJavascriptInterface"})
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        webView = new WebView(this);
        setContentView(webView);
        WebSettings s = webView.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        CookieManager.getInstance().setAcceptCookie(true);
        CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true);
        webView.addJavascriptInterface(new Bridge(), "soliNativeHost");
        webView.setWebViewClient(new WebViewClient() {
            @Override public void onPageFinished(WebView view, String url) {
                registerTokenWhenReady();
            }
        });
        webView.loadUrl(START_URL);
    }

    void registerTokenWhenReady() {
        FirebaseMessaging.getInstance().getToken().addOnSuccessListener(token -> {
            new Thread(() -> postDevice("android", token)).start();
        });
    }

    void postDevice(String platform, String token) {
        try {
            String cookie = CookieManager.getInstance().getCookie(START_URL);
            if (cookie == null || cookie.length() < 8) {
                return;
            }
            String base = START_URL.replaceAll("/$", "");
            URL url = new URL(base + "/devices");
            HttpURLConnection c = (HttpURLConnection) url.openConnection();
            c.setRequestMethod("POST");
            c.setDoOutput(true);
            c.setRequestProperty("Content-Type", "application/json");
            c.setRequestProperty("Cookie", cookie);
            // Same-origin CSRF gate accepts a matching Origin; skip_csrf on
            // /devices/* covers shells that omit it.
            c.setRequestProperty("Origin", base);
            c.setRequestProperty("Referer", base + "/");
            JSONObject body = new JSONObject();
            body.put("platform", platform);
            body.put("token", token);
            byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
            try (OutputStream os = c.getOutputStream()) { os.write(bytes); }
            c.getResponseCode();
            c.disconnect();
        } catch (Exception ignored) {}
    }

    public class Bridge {
        @JavascriptInterface
        public String capabilities() {
            return "notify,geolocation,vibrate,share,keep_awake,print,clipboard,badge,camera";
        }

        @JavascriptInterface
        public void notify(String json) {
            // Open-app path: also raised via FCM when backgrounded.
        }

        @JavascriptInterface
        public void call(String json) {}
    }
}
