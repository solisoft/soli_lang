// Soli Nova extension entry point.
//
// Starts the Soli language server (`soli lsp`) via Nova's LanguageClient API.
// The server speaks LSP over stdio; Nova handles framing, request routing,
// and surfacing diagnostics/completions/hover/etc. in the editor.

var langClient = null;

exports.activate = function () {
  if (!nova.workspace.config.get("soli.lsp.enabled", "boolean")) {
    return;
  }
  startServer();
};

exports.deactivate = function () {
  stopServer();
};

nova.commands.register("soli.restartServer", function () {
  stopServer();
  startServer();
});

// Re-spawn when the user changes settings that affect how we launch.
nova.workspace.config.onDidChange("soli.lsp.path", restartIfRunning);
nova.workspace.config.onDidChange("soli.lsp.enabled", function (enabled) {
  if (enabled) {
    startServer();
  } else {
    stopServer();
  }
});
nova.workspace.config.onDidChange("soli.lsp.trace", restartIfRunning);

function restartIfRunning() {
  if (langClient) {
    stopServer();
    startServer();
  }
}

function resolveServerPath() {
  var configured = nova.workspace.config.get("soli.lsp.path", "string");
  if (configured && configured.trim().length > 0) {
    return configured;
  }
  // Fall back to whichever `soli` is on PATH — Nova resolves bare names via
  // the user's login shell when `shell: true` is set on the Process.
  return "soli";
}

function startServer() {
  if (langClient) {
    return;
  }

  var serverOptions = {
    path: resolveServerPath(),
    args: ["lsp"],
    type: "stdio",
  };

  var clientOptions = {
    syntaxes: ["soli"],
    initializationOptions: {
      trace: nova.workspace.config.get("soli.lsp.trace", "string") || "off",
    },
  };

  var client = new LanguageClient(
    "soli",
    "Soli Language Server",
    serverOptions,
    clientOptions
  );

  try {
    client.start();
    nova.subscriptions.add(client);
    langClient = client;
  } catch (err) {
    console.error("Soli LSP failed to start:", err);
    if (nova.inDevMode()) {
      nova.workspace.showErrorMessage(
        "Could not start the Soli language server.\n\n" +
          "Check that `soli` is installed and that `soli lsp` runs.\n\n" +
          String(err)
      );
    }
  }
}

function stopServer() {
  if (!langClient) {
    return;
  }
  try {
    langClient.stop();
  } catch (err) {
    console.error("Soli LSP stop error:", err);
  }
  langClient = null;
}
