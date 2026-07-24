# {{APP_NAME}} — Windows shell (WebView2)

Minimal WinForms + WebView2 host for `{{START_URL}}`.

## Requirements

- Windows 10/11 with WebView2 Runtime
- .NET 8 SDK

```bash
dotnet build -c Release
dotnet run --project {{SCHEME}}Shell
```

Protocol registration (per-user):

```powershell
.\register-protocol.ps1
```

Then `{{SCHEME}}://pings/3` launches the shell with that path.
