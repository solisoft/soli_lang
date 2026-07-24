using System.Text.Json;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.WinForms;

namespace {{SCHEME}}Shell;

static class Program
{
    public const string StartUrl = "{{START_URL}}";
    public const string Scheme = "{{SCHEME}}";
    public const string Host = "{{HOST}}";

    [STAThread]
    static void Main(string[] args)
    {
        ApplicationConfiguration.Initialize();
        var open = MapDeepLink(args.ElementAtOrDefault(0));
        Application.Run(new MainForm(open ?? StartUrl));
    }

    public static string? MapDeepLink(string? arg)
    {
        if (string.IsNullOrWhiteSpace(arg)) return null;
        arg = arg.Trim();
        if (arg.StartsWith(Scheme + "://", StringComparison.OrdinalIgnoreCase))
        {
            var rest = arg[(Scheme.Length + 3)..];
            var slash = rest.IndexOf('/');
            var path = slash >= 0 ? rest[slash..] : "/";
            return StartUrl.TrimEnd('/') + path;
        }
        if (arg.StartsWith("https://", StringComparison.OrdinalIgnoreCase) ||
            arg.StartsWith("http://", StringComparison.OrdinalIgnoreCase))
            return arg;
        if (arg.StartsWith('/'))
            return StartUrl.TrimEnd('/') + arg;
        return null;
    }
}

sealed class MainForm : Form
{
    readonly WebView2 _web = new() { Dock = DockStyle.Fill };
    readonly string _start;

    public MainForm(string startUrl)
    {
        Text = "{{APP_NAME}}";
        Width = 420;
        Height = 780;
        _start = startUrl;
        Controls.Add(_web);
        Load += async (_, _) =>
        {
            await _web.EnsureCoreWebView2Async();
            _web.CoreWebView2.Settings.AreDefaultContextMenusEnabled = true;
            _web.CoreWebView2.Settings.IsStatusBarEnabled = false;
            await _web.CoreWebView2.AddScriptToExecuteOnDocumentCreatedAsync(
                "window.soli = window.soli || {};" +
                "window.soli.native = {" +
                "  platform: 'windows'," +
                "  capabilities: ['notify','share','print','clipboard','badge']," +
                "  notify: function(json) {" +
                "    window.chrome.webview.postMessage(typeof json === 'string' ? json : JSON.stringify(json));" +
                "  }," +
                "  call: function(json) {" +
                "    window.chrome.webview.postMessage(typeof json === 'string' ? json : JSON.stringify(json));" +
                "  }" +
                "};" +
                "window.soliNativeHost = {" +
                "  capabilities: function() { return 'notify,share,print,clipboard,badge'; }," +
                "  notify: function(json) { window.soli.native.notify(json); }," +
                "  call: function(json) { window.soli.native.call(json); }" +
                "};"
            );
            _web.CoreWebView2.WebMessageReceived += OnMessage;
            _web.CoreWebView2.Navigate(_start);
        };
    }

    void OnMessage(object? sender, CoreWebView2WebMessageReceivedEventArgs e)
    {
        try
        {
            var json = e.TryGetWebMessageAsString();
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (root.TryGetProperty("method", out _)) return;
            var title = root.TryGetProperty("title", out var t) ? t.GetString() : "{{APP_NAME}}";
            var body = root.TryGetProperty("body", out var b) ? b.GetString() : "";
            BeginInvoke(() => { Text = $"{title} — {body}"; });
        }
        catch { /* ignore */ }
    }
}
