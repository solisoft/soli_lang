package {{JAVA_PACKAGE}};

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.print.PrintAttributes;
import android.print.PrintManager;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.CookieManager;
import android.webkit.GeolocationPermissions;
import android.webkit.JavascriptInterface;
import android.webkit.PermissionRequest;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.ProgressBar;

import org.json.JSONObject;

/**
 * Thin WebView shell for a Soli deployment.
 * Bridge contract matches src/serve/native.js (soliNativeHost).
 */
public class MainActivity extends Activity {
    private static final String START_URL = "{{START_URL}}";
    private static final String[] IN_APP_HOSTS = { "{{HOST}}" };
    private static final String SCHEME = "{{SCHEME}}";
    private static final String CHANNEL = "{{SCHEME}}_notify";
    private static final String BADGE_CHANNEL = "{{SCHEME}}_badge";
    private static final int BADGE_ID = 424242;
    private static final int FILE_CHOOSER_REQUEST = 1;
    private static final int NOTIF_PERM = 2;
    private static final int ACCENT = 0xFFF59E0B;

    private WebView webView;
    private ProgressBar progress;
    private ValueCallback<Uri[]> pendingFile;
    private PermissionRequest pendingMedia;
    private String pendingLocationOrigin;
    private GeolocationPermissions.Callback pendingLocationCallback;
    private String pendingDeepLink;

    @Override
    @SuppressLint({"SetJavaScriptEnabled", "AddJavascriptInterface"})
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ensureChannels();
        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(new String[]{"android.permission.POST_NOTIFICATIONS"}, NOTIF_PERM);
        }

        FrameLayout root = new FrameLayout(this);
        webView = new WebView(this);
        progress = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progress.setLayoutParams(new FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 8));
        root.addView(webView, new FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        root.addView(progress);
        setContentView(root);

        WebSettings s = webView.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setMediaPlaybackRequiresUserGesture(false);
        s.setGeolocationEnabled(true);
        CookieManager.getInstance().setAcceptCookie(true);
        CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true);

        webView.addJavascriptInterface(new SoliNativeBridge(), "soliNativeHost");
        webView.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri uri = request.getUrl();
                String host = uri.getHost();
                if (host != null) {
                    for (String h : IN_APP_HOSTS) {
                        if (host.equals(h) || host.endsWith("." + h)) return false;
                    }
                }
                if (SCHEME.equals(uri.getScheme())) {
                    view.loadUrl(mapScheme(uri));
                    return true;
                }
                startActivity(new Intent(Intent.ACTION_VIEW, uri));
                return true;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                if (pendingDeepLink != null) {
                    String path = pendingDeepLink;
                    pendingDeepLink = null;
                    view.loadUrl(path);
                }
            }
        });
        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onProgressChanged(WebView view, int newProgress) {
                progress.setProgress(newProgress);
                progress.setVisibility(newProgress >= 100 ? android.view.View.GONE : android.view.View.VISIBLE);
            }

            @Override
            public void onPermissionRequest(PermissionRequest request) {
                pendingMedia = request;
                requestPermissions(request.getResources(), 3);
            }

            @Override
            public void onGeolocationPermissionsShowPrompt(String origin, GeolocationPermissions.Callback callback) {
                pendingLocationOrigin = origin;
                pendingLocationCallback = callback;
                requestPermissions(new String[]{"android.permission.ACCESS_FINE_LOCATION"}, 4);
            }

            @Override
            public boolean onShowFileChooser(WebView w, ValueCallback<Uri[]> filePathCallback, FileChooserParams params) {
                pendingFile = filePathCallback;
                Intent i = params.createIntent();
                try {
                    startActivityForResult(i, FILE_CHOOSER_REQUEST);
                } catch (Exception e) {
                    pendingFile = null;
                    return false;
                }
                return true;
            }
        });

        handleIntent(getIntent());
        if (pendingDeepLink == null) {
            webView.loadUrl(START_URL);
        } else {
            webView.loadUrl(pendingDeepLink);
            pendingDeepLink = null;
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
        if (pendingDeepLink != null && webView != null) {
            webView.loadUrl(pendingDeepLink);
            pendingDeepLink = null;
        }
    }

    private void handleIntent(Intent intent) {
        if (intent == null || intent.getData() == null) return;
        Uri uri = intent.getData();
        if (SCHEME.equals(uri.getScheme())) {
            pendingDeepLink = mapScheme(uri);
        } else if ("https".equals(uri.getScheme())) {
            pendingDeepLink = uri.toString();
        }
    }

    private String mapScheme(Uri uri) {
        String path = uri.getPath();
        if (path == null || path.isEmpty()) path = "/";
        String q = uri.getQuery();
        String base = START_URL.endsWith("/") ? START_URL.substring(0, START_URL.length() - 1) : START_URL;
        return base + path + (q != null ? "?" + q : "");
    }

    private void ensureChannels() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "{{APP_NAME}}", NotificationManager.IMPORTANCE_DEFAULT));
        NotificationChannel badge = new NotificationChannel(BADGE_CHANNEL, "Badge", NotificationManager.IMPORTANCE_MIN);
        badge.setShowBadge(true);
        nm.createNotificationChannel(badge);
    }

    @Override
    public void onBackPressed() {
        if (webView != null && webView.canGoBack()) webView.goBack();
        else super.onBackPressed();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode == FILE_CHOOSER_REQUEST && pendingFile != null) {
            pendingFile.onReceiveValue(WebChromeClient.FileChooserParams.parseResult(resultCode, data));
            pendingFile = null;
        }
        super.onActivityResult(requestCode, resultCode, data);
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        if (requestCode == 3 && pendingMedia != null) {
            boolean ok = grantResults.length > 0 && grantResults[0] == android.content.pm.PackageManager.PERMISSION_GRANTED;
            if (ok) pendingMedia.grant(pendingMedia.getResources());
            else pendingMedia.deny();
            pendingMedia = null;
        }
        if (requestCode == 4 && pendingLocationCallback != null) {
            boolean ok = grantResults.length > 0 && grantResults[0] == android.content.pm.PackageManager.PERMISSION_GRANTED;
            pendingLocationCallback.invoke(pendingLocationOrigin, ok, false);
            pendingLocationCallback = null;
        }
    }

    private void reply(final String id, final Object result, final String error) {
        if (id == null || id.isEmpty()) return;
        final String js;
        try {
            JSONObject o = new JSONObject();
            o.put("id", id);
            if (error != null) o.put("error", error);
            else o.put("result", result == null ? JSONObject.NULL : result);
            js = "window.__soliNativeReply && window.__soliNativeReply(" + JSONObject.quote(o.toString()) + ")";
        } catch (Exception e) {
            return;
        }
        runOnUiThread(() -> webView.evaluateJavascript(js, null));
    }

    public class SoliNativeBridge {
        @JavascriptInterface
        public String capabilities() {
            return "notify,geolocation,vibrate,share,keep_awake,print,clipboard,badge,camera,nfc,biometric";
        }

        @JavascriptInterface
        public void notify(String json) {
            try {
                JSONObject o = new JSONObject(json);
                String title = o.optString("title", "{{APP_NAME}}");
                String body = o.optString("body", "");
                String url = o.optString("url", START_URL);
                Intent open = new Intent(MainActivity.this, MainActivity.class);
                open.setData(Uri.parse(url.startsWith("http") ? url : START_URL.replaceAll("/$", "") + url));
                open.setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
                PendingIntent pi = PendingIntent.getActivity(
                    MainActivity.this, (int) System.currentTimeMillis(), open,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
                Notification.Builder b = Build.VERSION.SDK_INT >= 26
                    ? new Notification.Builder(MainActivity.this, CHANNEL)
                    : new Notification.Builder(MainActivity.this);
                b.setContentTitle(title).setContentText(body).setSmallIcon(android.R.drawable.ic_dialog_info)
                    .setContentIntent(pi).setAutoCancel(true).setColor(ACCENT);
                getSystemService(NotificationManager.class).notify((int) System.currentTimeMillis(), b.build());
            } catch (Exception ignored) {}
        }

        @JavascriptInterface
        public void call(String json) {
            try {
                JSONObject o = new JSONObject(json);
                String id = o.optString("id", "");
                String method = o.optString("method", "");
                JSONObject params = o.optJSONObject("params");
                if (params == null) params = new JSONObject();
                switch (method) {
                    case "vibrate": {
                        Vibrator v = (Vibrator) getSystemService(Context.VIBRATOR_SERVICE);
                        if (v != null && Build.VERSION.SDK_INT >= 26) {
                            v.vibrate(VibrationEffect.createOneShot(params.optLong("pattern", 200), VibrationEffect.DEFAULT_AMPLITUDE));
                        }
                        reply(id, true, null);
                        break;
                    }
                    case "share": {
                        Intent send = new Intent(Intent.ACTION_SEND);
                        send.setType("text/plain");
                        send.putExtra(Intent.EXTRA_TEXT, params.optString("url", params.optString("text", "")));
                        send.putExtra(Intent.EXTRA_TITLE, params.optString("title", ""));
                        startActivity(Intent.createChooser(send, params.optString("title", "Share")));
                        reply(id, true, null);
                        break;
                    }
                    case "badge": {
                        int count = params.optInt("count", 0);
                        NotificationManager nm = getSystemService(NotificationManager.class);
                        if (count <= 0) {
                            nm.cancel(BADGE_ID);
                        } else {
                            Notification.Builder b = Build.VERSION.SDK_INT >= 26
                                ? new Notification.Builder(MainActivity.this, BADGE_CHANNEL)
                                : new Notification.Builder(MainActivity.this);
                            b.setContentTitle("{{APP_NAME}}").setContentText(" ").setSmallIcon(android.R.drawable.ic_dialog_info)
                                .setNumber(count).setOngoing(true);
                            nm.notify(BADGE_ID, b.build());
                        }
                        reply(id, count, null);
                        break;
                    }
                    case "keep_awake": {
                        boolean on = params.optBoolean("on", true);
                        runOnUiThread(() -> {
                            if (on) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                            else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                        });
                        reply(id, on, null);
                        break;
                    }
                    case "print": {
                        runOnUiThread(() -> {
                            PrintManager pm = (PrintManager) getSystemService(Context.PRINT_SERVICE);
                            pm.print("{{APP_NAME}}", webView.createPrintDocumentAdapter("{{APP_NAME}}"), new PrintAttributes.Builder().build());
                        });
                        reply(id, true, null);
                        break;
                    }
                    case "clipboard": {
                        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                        if (params.has("text")) {
                            cm.setPrimaryClip(ClipData.newPlainText("text", params.optString("text", "")));
                            reply(id, true, null);
                        } else {
                            reply(id, "", null);
                        }
                        break;
                    }
                    default:
                        reply(id, null, "unsupported: " + method);
                }
            } catch (Exception e) {
                // ignore
            }
        }
    }
}
